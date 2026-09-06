from __future__ import annotations

from typing import Annotated, Literal

from pydantic import BaseModel, ConfigDict, Field, model_validator


MAX_ACTIONS_PER_STEP = 16
Coordinate = Annotated[float, Field(ge=0, le=1, allow_inf_nan=False)]
KeyName = Annotated[str, Field(
    min_length=1,
    pattern=r"^(\S+| )$",
    description=(
        "A single literal character (including a space), or a named key: "
        "ctrl, shift, alt, meta, enter, tab, space, backspace, delete, escape, "
        "home, end, pageup, pagedown, left, right, up, down, f1 through f12. "
        "Use meta for Command on macOS."
    ),
)]


class Action(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True)


class Click(Action):
    type: Literal["click", "double_click"]
    x: Coordinate
    y: Coordinate
    button: Literal["left", "right", "middle"]


class ClickElement(Action):
    type: Literal["click_element"]
    element_id: int = Field(ge=1)
    button: Literal["left", "right", "middle"] = "left"


class SetValue(Action):
    type: Literal["set_value"]
    element_id: int = Field(ge=1)
    text: str


class Drag(Action):
    type: Literal["drag"]
    from_x: Coordinate
    from_y: Coordinate
    to_x: Coordinate
    to_y: Coordinate
    duration_ms: int = Field(ge=40, le=2000)
    button: Literal["left", "right", "middle"]


class Scroll(Action):
    type: Literal["scroll"]
    horizontal: int = Field(ge=-30, le=30)
    vertical: int = Field(ge=-30, le=30)


class TypeText(Action):
    type: Literal["type"]
    text: str


class Key(Action):
    type: Literal["key"]
    key: KeyName


class Hotkey(Action):
    type: Literal["hotkey"]
    keys: list[KeyName] = Field(min_length=1, max_length=5)


class Wait(Action):
    type: Literal["wait"]
    milliseconds: int = Field(ge=1, le=5000)


class ComputerDecision(BaseModel):
    """Typed model output using the native host's action protocol directly."""

    model_config = ConfigDict(extra="forbid", strict=True)

    status: Literal["continue", "done", "blocked"]
    actions: list[ClickElement | SetValue | Click | Drag | Scroll | TypeText | Key | Hotkey | Wait] = Field(
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
