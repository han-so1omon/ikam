"""Authored graph-boundary declarations for operator-facing workflow nodes."""

from __future__ import annotations

from pydantic import BaseModel, ConfigDict, Field


class OperatorBoundarySpec(BaseModel):
    name: str = Field(min_length=1)
    mime_type: str = Field(min_length=1)

    model_config = ConfigDict(extra="forbid")


class OperatorBoundaries(BaseModel):
    input: list[OperatorBoundarySpec] = Field(default_factory=list)
    output: list[OperatorBoundarySpec] = Field(default_factory=list)

    model_config = ConfigDict(extra="forbid")
