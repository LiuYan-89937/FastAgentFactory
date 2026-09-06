from __future__ import annotations

from typing import Annotated, Literal

from pydantic import BaseModel, ConfigDict, Field, model_validator


MAX_ACTIONS_PER_STEP = 16
AccessibilityActionName = Annotated[str, Field(min_length=1)]


class Action(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True)


class PerformAction(Action):
    type: Literal["perform_action"]
    element_id: int = Field(ge=1)
    action: AccessibilityActionName


class SetValue(Action):
    type: Literal["set_value"]
    element_id: int = Field(ge=1)
    text: str


class Wait(Action):
    type: Literal["wait"]
    milliseconds: int = Field(ge=1, le=5000)


class ComputerDecision(BaseModel):
    """Typed model output using the native host's action protocol directly."""

    model_config = ConfigDict(extra="forbid", strict=True)

    status: Literal["continue", "done", "blocked"]
    actions: list[PerformAction | SetValue | Wait] = Field(
        max_length=MAX_ACTIONS_PER_STEP,
    )
    note: str

    @model_validator(mode="after")
    def validate_status_actions(self) -> "ComputerDecision":
        if self.status == "continue" and not self.actions:
            raise ValueError("continue requires at least one action")
        if self.status == "blocked" and self.actions:
            raise ValueError("blocked decisions must not contain actions")
        return self


class ApplicationSelection(BaseModel):
    """Select exactly one application before opening a targeted computer session."""

    model_config = ConfigDict(extra="forbid", strict=True)

    status: Literal["selected", "blocked"]
    application_id: str | None = None
    note: str

    @model_validator(mode="after")
    def validate_selection(self) -> "ApplicationSelection":
        selected = bool(str(self.application_id or "").strip())
        if self.status == "selected" and not selected:
            raise ValueError("selected requires application_id")
        if self.status == "blocked" and selected:
            raise ValueError("blocked must not include application_id")
        return self
