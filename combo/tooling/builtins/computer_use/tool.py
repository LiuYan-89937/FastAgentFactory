from __future__ import annotations

from typing import Any

from combo.computer_use import RuntimeComputerUse
from combo.tooling.envelope import tool_envelope
from combo.tooling.execution_context import current_tool_call, current_tool_event_sink


def run(arguments: dict[str, Any], resources: dict[str, Any]) -> dict[str, Any]:
    runtime = resources.get("computer_use_runtime")
    if not isinstance(runtime, RuntimeComputerUse):
        raise RuntimeError("computer-use runtime resource is not configured")
    goal = str(arguments.get("goal") or "").strip()
    if not goal:
        raise ValueError("computer_use goal must not be empty")
    result = runtime.run(goal=goal, on_progress=_progress_observer())
    return tool_envelope(result, summary=f"computer_use {result['status']}")


def _progress_observer():
    current = current_tool_call()
    sink = current_tool_event_sink()
    if current is None or sink is None:
        return None

    def publish(progress: dict[str, Any]) -> None:
        sink(
            {
                "event_type": "tool_output_delta",
                "tool_id": current.tool_id,
                "tool_call_id": current.tool_call_id,
                "status": "running",
                "output": progress,
                "message": str(progress.get("message") or ""),
            }
        )

    return publish
