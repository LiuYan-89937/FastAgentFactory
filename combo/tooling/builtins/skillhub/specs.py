from __future__ import annotations

from combo.tooling.skillhub.search_query import (
    SKILLHUB_SEARCH_QUERY_MAX_CHARS,
    SKILLHUB_SEARCH_QUERY_PATTERN,
)
from combo.tooling.spec import ToolRiskEvaluatorConfig, ToolSpec


SKILLHUB_RUNTIME_RESOURCE = "skillhub_runtime"
RUNTIME_IDENTITY_RESOURCE = "runtime_identity"


def get_skillhub_tool_specs() -> list[ToolSpec]:
    return [
        ToolSpec(
            id="skillhub",
            description=(
                "Search SkillHub and install or remove Skills in the unified Skill pool. "
                "Only the main Agent can use this capability-management tool."
            ),
            entrypoint="combo.tooling.builtins.skillhub.tool:run",
            input_schema=_input_schema(),
            output_schema={"type": "object"},
            resources={
                SKILLHUB_RUNTIME_RESOURCE: SKILLHUB_RUNTIME_RESOURCE,
                RUNTIME_IDENTITY_RESOURCE: RUNTIME_IDENTITY_RESOURCE,
            },
            risk_level="high",
            risk_evaluator=ToolRiskEvaluatorConfig(
                hard="combo.tooling.builtins.skillhub.tool:evaluate_risk",
            ),
            concurrent=False,
            max_parallel_calls=1,
            effects=["read", "write", "delete", "network", "process", "external_side_effect"],
            system_available=True,
        )
    ]


def _input_schema() -> dict:
    return {
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["status", "search", "install", "remove"],
                "description": "要执行的 SkillHub 操作。",
            },
            "query": {
                "type": "string",
                "minLength": 1,
                "maxLength": SKILLHUB_SEARCH_QUERY_MAX_CHARS,
                "pattern": SKILLHUB_SEARCH_QUERY_PATTERN,
                "description": "search 操作必填；一至三个简短能力关键词。",
            },
            "skill": {
                "type": "string",
                "minLength": 1,
                "description": "install/remove 操作必填；搜索结果返回的 install_name 或已安装 Skill 标识。",
            },
        },
        "required": ["action"],
        "additionalProperties": False,
    }
