"""Canonical executor declaration contracts for orchestration."""

from __future__ import annotations

from typing import Any

from pydantic import BaseModel, ConfigDict, Field, model_validator


class ExecutorDeclaration(BaseModel):
    executor_id: str = Field(min_length=1)
    executor_kind: str = Field(min_length=1)
    capabilities: list[str] = Field(min_length=1)
    policy_support: list[str]
    transport: dict[str, Any]
    runtime: dict[str, Any]
    concurrency: dict[str, Any]
    batching: dict[str, Any]
    health: dict[str, Any]

    model_config = ConfigDict(extra="forbid")

    @model_validator(mode="after")
    def validate_sections(self) -> "ExecutorDeclaration":
        _require_non_empty_section("transport", self.transport)
        _require_non_empty_section("runtime", self.runtime)
        _require_non_empty_section("concurrency", self.concurrency)
        _require_non_empty_section("batching", self.batching)
        _require_non_empty_section("health", self.health)
        if not isinstance(self.transport.get("kind"), str) or not self.transport["kind"].strip():
            raise ValueError("transport.kind is required")
        if not isinstance(self.health.get("readiness_path"), str) or not self.health["readiness_path"].strip():
            raise ValueError("health.readiness_path is required")
        return self


def _require_non_empty_section(name: str, value: dict[str, Any]) -> None:
    if not value:
        raise ValueError(f"{name} section must not be empty")
