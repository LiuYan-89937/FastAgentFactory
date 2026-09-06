from __future__ import annotations

from dataclasses import dataclass, replace
import logging
import json
from threading import Event, RLock
from time import perf_counter
from typing import Any, Callable

from langchain_core.callbacks import UsageMetadataCallbackHandler
from langchain_core.messages import HumanMessage, SystemMessage

from combo.computer_use.decisions import ApplicationSelection, ComputerDecision
from combo.computer_use.host import (
    ApplicationDescriptor,
    ApplicationTarget,
    ComputerHostClient,
    WindowObservation,
)
from combo.dynamic_runtime.model_service import RuntimeModelResolver
from combo.models.chat_model import create_chat_model_from_settings
from combo.runtime_kernel.model_operations import prepare_structured_output_invocation
from combo.runtime_protocol import RuntimeInstance
from combo.tooling.execution_context import (
    RuntimeToolExecutionCancelled,
    register_runtime_tool_cancellation,
    runtime_terminal_cancellation_requested,
    runtime_tool_interruption_requested,
)


MAX_COMPUTER_STEPS = 32
COMPUTER_MODEL_MAX_OUTPUT_TOKENS = 700

_APPLICATION_SELECTION_PROMPT = """Select the single application that must be controlled to finish GOAL.
Only choose an application_id present in APPLICATIONS. Use window titles to disambiguate.
Return blocked only when none of the listed applications can satisfy the goal.
GOAL: {goal}"""

_COMPUTER_PROMPT = """You control one attached application window. Finish GOAL quickly.
Return the structured decision with status, actions and a short note.
Each observation contains only the attached application's accessibility tree. There are no pixels or coordinate actions.
Use perform_action only with an element_id and an action explicitly listed on that node. Use set_value only when the node lists set_value. Element IDs belong only to the current observation; after an action changes the interface, stop the batch and observe again.
Batch only deterministic actions that cannot invalidate later element paths. A done decision may include the final deterministic action that completes GOAL; the runtime executes it before completing. Use done without actions only when GOAL is already semantically complete.
GOAL: {goal}"""


_logger = logging.getLogger(__name__)
ComputerUseProgressObserver = Callable[[dict[str, Any]], None]


@dataclass(frozen=True, slots=True)
class ComputerUseResult:
    status: str
    summary: str
    steps: int
    model_calls: int
    total_tokens: int
    application: dict[str, Any] | None = None

    def payload(self) -> dict[str, Any]:
        payload = {
            "status": self.status,
            "summary": self.summary,
            "steps": self.steps,
            "model_calls": self.model_calls,
            "total_tokens": self.total_tokens,
        }
        if self.application is not None:
            payload["application"] = self.application
        return payload


class ComputerUseCoordinator:
    """Own the high-speed vision/action loop without entering the ordinary tool loop."""

    def __init__(self, *, model_resolver: RuntimeModelResolver, host: ComputerHostClient | None) -> None:
        self._model_resolver = model_resolver
        self._host = host
        self._activity_lock = RLock()
        self._active_request_id: str | None = None

    @classmethod
    def from_environment(cls, *, model_resolver: RuntimeModelResolver) -> "ComputerUseCoordinator":
        return cls(model_resolver=model_resolver, host=ComputerHostClient.from_environment())

    def for_runtime(self, instance: RuntimeInstance) -> "RuntimeComputerUse":
        return RuntimeComputerUse(
            coordinator=self,
            instance=instance,
        )

    def close(self) -> None:
        if self._host is not None:
            try:
                self._host.cancel_session()
            finally:
                self._host.close()

    def _run(
        self,
        *,
        instance: RuntimeInstance,
        goal: str,
        on_progress: ComputerUseProgressObserver | None = None,
    ) -> ComputerUseResult:
        request_id = instance.request.request_id
        with self._activity_lock:
            if self._active_request_id is not None:
                raise RuntimeError(
                    "Computer Use is already active for request "
                    f"{self._active_request_id}"
                )
            self._active_request_id = request_id
        cancelled = Event()
        active_session_id: list[str | None] = [None]

        def cancel_active_session() -> None:
            cancelled.set()
            session_id = active_session_id[0]
            if self._host is not None and session_id is not None:
                try:
                    self._host.cancel_session(session_id)
                except Exception:
                    _logger.exception(
                        "Computer use request=%s native cancellation failed",
                        request_id,
                    )
            with self._activity_lock:
                if self._active_request_id == request_id:
                    self._active_request_id = None

        unregister_cancellation = register_runtime_tool_cancellation(cancel_active_session)
        try:
            _ensure_not_cancelled(cancelled, self._host, active_session_id[0])
            return self._run_exclusive(
                instance=instance,
                goal=goal,
                on_progress=on_progress,
                cancelled=cancelled,
                active_session_id=active_session_id,
            )
        finally:
            unregister_cancellation()
            with self._activity_lock:
                if self._active_request_id == request_id:
                    self._active_request_id = None

    def _run_exclusive(
        self,
        *,
        instance: RuntimeInstance,
        goal: str,
        on_progress: ComputerUseProgressObserver | None,
        cancelled: Event,
        active_session_id: list[str | None],
    ) -> ComputerUseResult:
        if instance.request.runtime_role != "main":
            raise PermissionError("system computer use is available only to the main runtime")
        host = self._host
        if host is None:
            raise RuntimeError(
                "system computer use requires the Combo desktop native host; it is unavailable in this backend process"
            )
        phase_started = perf_counter()
        frozen = instance.request.policy_snapshot.model
        resolved = self._model_resolver.resolve_chat_model(
            operation="computer_use",
            profile_id=frozen.profile_id,
            expected_profile_revision=frozen.profile_revision,
            expected_credential_revision=frozen.credential_revision,
            reasoning_intensity=1,
        )
        configured_max = resolved.settings.max_output_tokens
        max_output = (
            min(configured_max, COMPUTER_MODEL_MAX_OUTPUT_TOKENS)
            if configured_max is not None
            else COMPUTER_MODEL_MAX_OUTPUT_TOKENS
        )
        model = create_chat_model_from_settings(
            replace(
                resolved.settings,
                role="computer_use",
                max_output_tokens=max_output,
            )
        )
        if model is None:
            raise RuntimeError("computer-use model could not be created from the frozen profile")
        _ensure_not_cancelled(cancelled, host, active_session_id[0])
        _publish_progress(on_progress, phase="model_setup", message="Desktop model is ready.")

        _logger.info("Computer use request=%s phase=model_setup elapsed_ms=%.1f",
                     instance.request.request_id, (perf_counter() - phase_started) * 1000)
        normalized_goal = str(goal or "").strip()
        if not normalized_goal:
            raise ValueError("computer_use goal must not be empty")
        model_calls = 0
        total_tokens = 0
        last_note = ""

        usage_callback = UsageMetadataCallbackHandler()
        phase_started = perf_counter()
        _ensure_not_cancelled(cancelled, host, active_session_id[0])
        session_id = host.start()
        active_session_id[0] = session_id
        failed = False
        try:
            _ensure_not_cancelled(cancelled, host, session_id)
            _publish_progress(
                on_progress,
                phase="applications",
                message="Reading available applications.",
            )
            applications = host.list_applications(session_id)
            _ensure_not_cancelled(cancelled, host, session_id)
            if not applications:
                return ComputerUseResult(
                    status="blocked",
                    summary="No controllable application windows are available.",
                    steps=0,
                    model_calls=0,
                    total_tokens=0,
                )
            selection_invocation = prepare_structured_output_invocation(
                model=model,
                output_model=ApplicationSelection,
                messages=[
                    SystemMessage(
                        content=_APPLICATION_SELECTION_PROMPT.format(goal=normalized_goal)
                    ),
                    HumanMessage(content=_applications_message(applications)),
                ],
                model_metadata=resolved.settings.metadata(),
                config_tags=["computer-use", "application-selection"],
            )
            selection_response = selection_invocation.model.invoke(
                list(selection_invocation.messages),
                config={
                    "callbacks": [usage_callback],
                    "metadata": {
                        "operation": "computer_use_application_selection",
                        "request_id": instance.request.request_id,
                    },
                },
            )
            _ensure_not_cancelled(cancelled, host, session_id)
            selection = ApplicationSelection.model_validate(selection_response)
            model_calls += 1
            total_tokens = _usage_total(usage_callback)
            if selection.status == "blocked":
                return ComputerUseResult(
                    status="blocked",
                    summary=selection.note or "No listed application can satisfy this task.",
                    steps=0,
                    model_calls=model_calls,
                    total_tokens=total_tokens,
                )
            application_id = str(selection.application_id or "").strip()
            if application_id not in {item.application_id for item in applications}:
                raise RuntimeError(
                    "computer-use model selected an application outside the current application list"
                )
            _publish_progress(
                on_progress,
                phase="attaching",
                message="Opening the target application.",
            )
            _ensure_not_cancelled(cancelled, host, session_id)
            target = host.attach_application(session_id, application_id)
            _ensure_not_cancelled(cancelled, host, session_id)
            _logger.info(
                "Computer use request=%s selected application_id=%s bundle_id=%s pid=%s window_id=%s",
                instance.request.request_id,
                target.application_id,
                target.bundle_identifier,
                target.process_id,
                target.window_id,
            )
            system = SystemMessage(content=_COMPUTER_PROMPT.format(goal=normalized_goal))
            _publish_progress(
                on_progress,
                phase="observing",
                message="Reading the target window.",
                target=target,
            )
            observation = host.observe(session_id)
            _ensure_not_cancelled(cancelled, host, session_id)
            _require_usable_observation(observation)
            _log_observation(instance.request.request_id, observation)
            _logger.info("Computer use request=%s phase=first_observation elapsed_ms=%.1f",
                         instance.request.request_id, (perf_counter() - phase_started) * 1000)
            for step in range(1, MAX_COMPUTER_STEPS + 1):
                _ensure_not_cancelled(cancelled, host, session_id)
                _publish_progress(
                    on_progress,
                    phase="analyzing",
                    step=step,
                    message="Analyzing the target window.",
                    observation=observation,
                )
                invocation = prepare_structured_output_invocation(
                    model=model,
                    output_model=ComputerDecision,
                    messages=[
                        system,
                        _observation_message(
                            observation,
                            last_note=last_note,
                        ),
                    ],
                    model_metadata=resolved.settings.metadata(),
                    config_tags=["computer-use"],
                )
                phase_started = perf_counter()
                try:
                    response = invocation.model.invoke(
                        list(invocation.messages),
                        config={
                            "callbacks": [usage_callback],
                            "metadata": {
                                "operation": "computer_use",
                                "request_id": instance.request.request_id,
                                "step": step,
                            },
                        },
                    )
                    _ensure_not_cancelled(cancelled, host, session_id)
                    decision = ComputerDecision.model_validate(response)
                finally:
                    _logger.info(
                        "Computer use request=%s step=%s phase=model_decision elapsed_ms=%.1f",
                        instance.request.request_id, step, (perf_counter() - phase_started) * 1000,
                    )
                model_calls += 1
                total_tokens = _usage_total(usage_callback)
                status = decision.status
                note = decision.note
                actions = [action.model_dump() for action in decision.actions]
                if status == "done" and not actions:
                    return ComputerUseResult(
                        status="completed",
                        summary=note or "Desktop task completed.",
                        steps=step,
                        model_calls=model_calls,
                        total_tokens=total_tokens,
                        application=_application_result(observation.target),
                    )
                if status == "blocked":
                    return ComputerUseResult(
                        status="blocked",
                        summary=note or "Desktop task requires user intervention.",
                        steps=step,
                        model_calls=model_calls,
                        total_tokens=total_tokens,
                        application=_application_result(observation.target),
                    )
                phase_started = perf_counter()
                _publish_progress(
                    on_progress,
                    phase="acting",
                    step=step,
                    action_count=len(actions),
                    message="Controlling the target window.",
                )
                _ensure_not_cancelled(cancelled, host, session_id)
                host.act(session_id, actions)
                _ensure_not_cancelled(cancelled, host, session_id)
                last_note = _compact_action_note(actions, note)
                _publish_progress(
                    on_progress,
                    phase="observing",
                    step=step,
                    message="Reading the updated window.",
                    target=observation.target,
                )
                observation = host.observe(session_id)
                _ensure_not_cancelled(cancelled, host, session_id)
                _require_usable_observation(observation)
                _log_observation(instance.request.request_id, observation)
                _logger.info("Computer use request=%s step=%s phase=actions_and_observe elapsed_ms=%.1f",
                             instance.request.request_id, step, (perf_counter() - phase_started) * 1000)
                if status == "done":
                    return ComputerUseResult(
                        status="completed",
                        summary=note or "Desktop task completed.",
                        steps=step,
                        model_calls=model_calls,
                        total_tokens=total_tokens,
                        application=_application_result(observation.target),
                    )
            return ComputerUseResult(
                status="step_limit",
                summary="Desktop task did not finish within the computer-use step limit.",
                steps=MAX_COMPUTER_STEPS,
                model_calls=model_calls,
                total_tokens=total_tokens,
                application=_application_result(observation.target),
            )
        except BaseException:
            failed = True
            raise
        finally:
            try:
                host.stop(session_id)
            except Exception:
                if not failed:
                    raise
                _logger.exception("Computer use cleanup failed after execution error")


@dataclass(frozen=True, slots=True)
class RuntimeComputerUse:
    coordinator: ComputerUseCoordinator
    instance: RuntimeInstance

    def run(
        self,
        *,
        goal: str,
        on_progress: ComputerUseProgressObserver | None = None,
    ) -> dict[str, Any]:
        return self.coordinator._run(
            instance=self.instance,
            goal=goal,
            on_progress=on_progress,
        ).payload()


def _ensure_not_cancelled(
    cancelled: Event,
    host: ComputerHostClient | None,
    session_id: str | None,
) -> None:
    if not (
        cancelled.is_set()
        or runtime_terminal_cancellation_requested()
        or runtime_tool_interruption_requested()
    ):
        return
    cancelled.set()
    if host is not None and session_id is not None:
        try:
            host.cancel_session(session_id)
        except Exception:
            _logger.exception("Computer use native cancellation failed")
    raise RuntimeToolExecutionCancelled("Computer Use execution was cancelled.")


def _observation_message(
    observation: WindowObservation,
    *,
    last_note: str,
) -> HumanMessage:
    state = {
        "application": observation.target.display_name,
        "window_title": observation.target.window_title,
        "last_action": last_note[:180],
        "accessibility": _model_accessibility(observation.accessibility),
    }
    return HumanMessage(content=json.dumps(state, ensure_ascii=False, separators=(",", ":")))


def _model_accessibility(accessibility: dict[str, Any]) -> dict[str, Any]:
    nodes = accessibility.get("nodes")
    compact_nodes = []
    for node in nodes if isinstance(nodes, list) else []:
        if not isinstance(node, dict):
            continue
        compact = {
            "id": node.get("element_id"),
            "parent": node.get("parent_id"),
            "role": str(node.get("role") or "").removeprefix("AX"),
        }
        for source, target in (
            ("subrole", "subrole"),
            ("name", "name"),
            ("value", "value"),
            ("identifier", "identifier"),
            ("placeholder", "placeholder"),
        ):
            value = str(node.get(source) or "").strip()
            if value:
                compact[target] = value
        actions = [
            str(action)
            for action in (node.get("actions") or [])
            if str(action).strip()
        ]
        if node.get("value_settable") is True:
            actions.append("set_value")
        if actions:
            compact["actions"] = actions
        for field in ("focused", "selected", "expanded"):
            if node.get(field) is True:
                compact[field] = True
        if node.get("enabled") is False:
            compact["enabled"] = False
        compact_nodes.append(compact)
    return {
        "complete": accessibility.get("complete") is True,
        "nodes": compact_nodes,
    }


def _require_usable_observation(observation: WindowObservation) -> None:
    accessibility = observation.accessibility
    accessibility_available = bool(
        isinstance(accessibility, dict)
        and accessibility.get("usable") is True
    )
    if not accessibility_available:
        error = (
            accessibility.get("error")
            if isinstance(accessibility, dict)
            else None
        )
        detail = str(error or "accessibility tree is unavailable")
        raise RuntimeError(f"computer use requires an accessibility tree: {detail}")


def _log_observation(
    request_id: str,
    observation: WindowObservation,
) -> None:
    accessibility = observation.accessibility
    _logger.info(
        "Computer use request=%s application_id=%s bundle_id=%s pid=%s window_id=%s ax_usable=%s ax_complete=%s ax_nodes=%s ax_actionable=%s ax_named=%s ax_quality=%s",
        request_id,
        observation.target.application_id,
        observation.target.bundle_identifier,
        observation.target.process_id,
        observation.target.window_id,
        accessibility.get("usable"),
        accessibility.get("complete"),
        len(accessibility.get("nodes") or []),
        accessibility.get("actionable_node_count"),
        accessibility.get("named_node_count"),
        accessibility.get("quality_score"),
    )


def _publish_progress(
    observer: ComputerUseProgressObserver | None,
    *,
    phase: str,
    message: str,
    step: int | None = None,
    action_count: int | None = None,
    observation: WindowObservation | None = None,
    target: ApplicationTarget | None = None,
) -> None:
    if observer is None:
        return
    progress: dict[str, Any] = {
        "phase": phase,
        "message": message,
        "step": step,
        "action_count": action_count,
    }
    if observation is not None:
        target = observation.target
        progress["accessibility"] = observation.accessibility
    if target is not None:
        progress["target"] = {
            "application_id": target.application_id,
            "display_name": target.display_name,
            "bundle_identifier": target.bundle_identifier,
            "process_id": target.process_id,
            "icon_data_url": target.icon_data_url,
            "window_id": target.window_id,
            "window_title": target.window_title,
        }
    observer(progress)


def _applications_message(applications: tuple[ApplicationDescriptor, ...]) -> str:
    payload = [
        {
            "application_id": application.application_id,
            "display_name": application.display_name,
            "bundle_identifier": application.bundle_identifier,
            "process_id": application.process_id,
            "windows": [
                {
                    "title": str(window.get("title") or ""),
                    "focused": bool(window.get("focused", False)),
                    "minimized": bool(window.get("minimized", False)),
                }
                for window in application.windows
            ],
        }
        for application in applications
    ]
    return json.dumps(
        {"applications": payload},
        ensure_ascii=False,
        separators=(",", ":"),
    )


def _application_result(target: ApplicationTarget) -> dict[str, Any]:
    return {
        "display_name": target.display_name,
        "bundle_identifier": target.bundle_identifier,
        "icon_data_url": target.icon_data_url,
    }


def _usage_total(callback: UsageMetadataCallbackHandler) -> int:
    return sum(
        int(usage.get("total_tokens") or 0)
        for usage in callback.usage_metadata.values()
    )


def _compact_action_note(actions: list[dict[str, Any]], note: str) -> str:
    names = ",".join(str(action.get("type") or "") for action in actions)
    return f"{names};{note}" if note else names
