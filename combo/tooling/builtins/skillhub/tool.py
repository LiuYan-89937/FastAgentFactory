from __future__ import annotations

from typing import Any

from combo.runtime_protocol import RuntimeExecutionIdentity
from combo.tooling.builtins.skillhub.specs import (
    RUNTIME_IDENTITY_RESOURCE,
    SKILLHUB_RUNTIME_RESOURCE,
)
from combo.tooling.envelope import tool_envelope
from combo.tooling.skillhub.search_query import normalize_skillhub_search_query
from combo.tooling.skillhub.service import SkillHubService


def evaluate_risk(arguments: dict[str, Any], context: dict[str, Any]) -> dict[str, Any]:
    action = str(arguments.get("action") or "").strip()
    if action in {"status", "search"}:
        return {"action": "allow", "risk_level": "low", "reasons": ["read-only SkillHub operation"]}
    if action in {"install", "remove"}:
        return {"action": "ask", "risk_level": "high", "reasons": [f"SkillHub {action} changes the unified Skill pool"]}
    return {"action": "deny", "risk_level": "high", "reasons": ["unsupported SkillHub action"]}


def run(arguments: dict[str, Any], resources: dict[str, Any]) -> dict[str, Any]:
    service = resources.get(SKILLHUB_RUNTIME_RESOURCE)
    identity = resources.get(RUNTIME_IDENTITY_RESOURCE)
    if not isinstance(service, SkillHubService):
        raise RuntimeError("SkillHub runtime is not configured")
    if not isinstance(identity, RuntimeExecutionIdentity) or identity.runtime_role != "main":
        raise PermissionError("SkillHub is available only to the main Agent")
    action = str(arguments.get("action") or "").strip()
    if action == "status":
        _reject_irrelevant_arguments(arguments, "query", "skill")
        output = service.status()
    elif action == "search":
        _reject_irrelevant_arguments(arguments, "skill")
        output = service.search(normalize_skillhub_search_query(_required_argument(arguments, "query")))
    elif action == "install":
        _reject_irrelevant_arguments(arguments, "query")
        output = service.install(_required_argument(arguments, "skill"))
    elif action == "remove":
        _reject_irrelevant_arguments(arguments, "query")
        output = service.remove(_required_argument(arguments, "skill"))
    else:
        raise ValueError(f"unsupported SkillHub action: {action}")
    return tool_envelope(output, summary=str(output.get("message") or f"SkillHub {action} completed"))


def _required_argument(arguments: dict[str, Any], name: str) -> str:
    value = str(arguments.get(name) or "").strip()
    if not value:
        raise ValueError(f"SkillHub {name} is required for this action")
    return value


def _reject_irrelevant_arguments(arguments: dict[str, Any], *names: str) -> None:
    unexpected = [name for name in names if arguments.get(name) not in (None, "")]
    if unexpected:
        raise ValueError(f"SkillHub action does not accept: {', '.join(unexpected)}")
