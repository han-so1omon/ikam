"""Shared transition validator contracts."""

from __future__ import annotations

from typing import Annotated, Literal

from pydantic import BaseModel, ConfigDict, Field


class RichPetriTypeValidatorConfig(BaseModel):
    schema: dict[str, object] = Field(min_length=1)

    model_config = ConfigDict(extra="forbid")


class RichPetriRegexValidatorConfig(BaseModel):
    pattern: str = Field(min_length=1)
    flags: str | None = Field(default=None, min_length=1)

    model_config = ConfigDict(extra="forbid")


class _RichPetriTransitionValidatorBase(BaseModel):
    name: str = Field(min_length=1)
    direction: Literal["input", "output"]
    selector: str = Field(min_length=1)
    target: str = Field(min_length=1)

    model_config = ConfigDict(extra="forbid")


class RichPetriTypeTransitionValidator(_RichPetriTransitionValidatorBase):
    kind: Literal["type"]
    config: RichPetriTypeValidatorConfig


class RichPetriRegexTransitionValidator(_RichPetriTransitionValidatorBase):
    kind: Literal["regex"]
    config: RichPetriRegexValidatorConfig


RichPetriTransitionValidator = Annotated[
    RichPetriTypeTransitionValidator | RichPetriRegexTransitionValidator,
    Field(discriminator="kind"),
]
