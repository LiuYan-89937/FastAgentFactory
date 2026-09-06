from __future__ import annotations

from combo.tooling.spec import ToolLoopPolicyConfig, ToolSpec


COMPUTER_USE_RUNTIME_RESOURCE = "computer_use_runtime"


def get_computer_use_tool_specs() -> list[ToolSpec]:
    return [
        ToolSpec(
            id="computer_use",
            description=(
                "Operate the user's macOS or Windows desktop through the system-level visual computer-use runtime. "
                "Use this for native desktop applications and whole-desktop interaction. It is independent from the "
                "browser_* built-in tools. Provide one concise goal; the computer-use runtime performs its own "
                "low-token vision/action loop and returns only the terminal result."
            ),
            entrypoint="combo.tooling.builtins.computer_use.tool:run",
            input_schema={
                "type": "object",
                "properties": {
                    "goal": {
                        "type": "string",
                        "minLength": 1,
                        "description": "A concise, complete desktop task objective for the visual computer-use loop.",
                    }
                },
                "required": ["goal"],
                "additionalProperties": False,
            },
            output_schema={
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "enum": ["completed", "blocked", "step_limit"],
                    },
                    "summary": {"type": "string"},
                    "steps": {"type": "integer"},
                    "final_frame_id": {"type": "integer"},
                    "model_calls": {"type": "integer"},
                    "total_tokens": {"type": "integer"},
                },
                "required": [
                    "status",
                    "summary",
                    "steps",
                    "final_frame_id",
                    "model_calls",
                    "total_tokens",
                ],
                "additionalProperties": False,
            },
            resources={"computer_use_runtime": COMPUTER_USE_RUNTIME_RESOURCE},
            risk_level="high",
            concurrent=False,
            max_parallel_calls=1,
            output_projection="passthrough",
            loop_policy=ToolLoopPolicyConfig(max_calls=4, max_identical_calls=2),
            effects=["external_side_effect"],
            read_only=False,
            system_available=True,
        )
    ]
