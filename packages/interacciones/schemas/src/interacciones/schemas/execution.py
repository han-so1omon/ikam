"""Canonical execution and approval contracts for orchestration."""

from __future__ import annotations

from enum import Enum
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field, model_validator


class ResolutionMode(str, Enum):
    """How an execution target is resolved."""

    CAPABILITY_POLICY = "capability_policy"
    DIRECT_EXECUTOR_REF = "direct_executor_ref"


class ExecutionScope(BaseModel):
    ref: str = Field(min_length=1)

    model_config = ConfigDict(extra="forbid")

    @model_validator(mode="after")
    def validate_ref(self) -> "ExecutionScope":
        normalized = self.ref.strip()
        if not normalized.startswith("refs/heads/"):
            raise ValueError("ref must be a git-style branch ref")
        suffix = normalized[len("refs/heads/") :]
        if not suffix or suffix.startswith("/") or suffix.endswith("/") or "//" in suffix:
            raise ValueError("ref must be a git-style branch ref")
        self.ref = normalized
        return self


class ExecutionRequest(BaseModel):
    request_id: str = Field(min_length=1)
    workflow_id: str = Field(min_length=1)
    step_id: str = Field(min_length=1)
    capability: str = Field(min_length=1)
    policy: dict[str, Any]
    constraints: dict[str, Any]
    payload: dict[str, Any]
    resolution_mode: ResolutionMode = ResolutionMode.CAPABILITY_POLICY
    direct_executor_ref: str | None = None

    model_config = ConfigDict(extra="forbid")

    @model_validator(mode="after")
    def validate_resolution(self) -> "ExecutionRequest":
        direct_executor_ref = (self.direct_executor_ref or "").strip()
        if self.resolution_mode == ResolutionMode.DIRECT_EXECUTOR_REF and not direct_executor_ref:
            raise ValueError("direct_executor_ref is required for direct_executor_ref resolution")
        if direct_executor_ref:
            self.direct_executor_ref = direct_executor_ref
        return self


class ExecutionQueueRequest(BaseModel):
    request_id: str = Field(min_length=1)
    workflow_id: str = Field(min_length=1)
    step_id: str = Field(min_length=1)
    executor_id: str = Field(min_length=1)
    executor_kind: str = Field(min_length=1)
    capability: str = Field(min_length=1)
    policy: dict[str, Any]
    constraints: dict[str, Any]
    payload: dict[str, Any]
    transport: dict[str, Any]

    model_config = ConfigDict(extra="forbid")


class ExecutionQueued(BaseModel):
    request_id: str = Field(min_length=1)
    workflow_id: str = Field(min_length=1)
    step_id: str = Field(min_length=1)
    executor_id: str = Field(min_length=1)
    executor_kind: str = Field(min_length=1)
    capability: str = Field(min_length=1)
    status: Literal["queued"] = "queued"

    model_config = ConfigDict(extra="forbid")


class ExecutionProgress(BaseModel):
    request_id: str = Field(min_length=1)
    workflow_id: str = Field(min_length=1)
    step_id: str = Field(min_length=1)
    executor_id: str = Field(min_length=1)
    status: str = Field(min_length=1)
    progress: float = Field(ge=0.0, le=1.0)
    message: str | None = None
    stdout_lines: list[str] = Field(default_factory=list)
    stderr_lines: list[str] = Field(default_factory=list)
    details: dict[str, Any] = Field(default_factory=dict)

    model_config = ConfigDict(extra="forbid")


class ExecutionCompleted(BaseModel):
    request_id: str = Field(min_length=1)
    workflow_id: str = Field(min_length=1)
    step_id: str = Field(min_length=1)
    executor_id: str = Field(min_length=1)
    result: dict[str, Any] = Field(default_factory=dict)
    artifacts: list[str] = Field(default_factory=list)
    stdout_lines: list[str] = Field(default_factory=list)
    stderr_lines: list[str] = Field(default_factory=list)
    model_config = ConfigDict(extra="forbid")


class ExecutionFailed(BaseModel):
    request_id: str = Field(min_length=1)
    workflow_id: str = Field(min_length=1)
    step_id: str = Field(min_length=1)
    executor_id: str = Field(min_length=1)
    error_code: str = Field(min_length=1)
    error_message: str = Field(min_length=1)
    retryable: bool = False
    stdout_lines: list[str] = Field(default_factory=list)
    stderr_lines: list[str] = Field(default_factory=list)
    details: dict[str, Any] = Field(default_factory=dict)

    model_config = ConfigDict(extra="forbid")


class ApprovalRequested(BaseModel):
    approval_id: str = Field(min_length=1)
    workflow_id: str = Field(min_length=1)
    step_id: str = Field(min_length=1)
    requested_by: str = Field(min_length=1)
    summary: str = Field(min_length=1)
    details: dict[str, Any] = Field(default_factory=dict)

    model_config = ConfigDict(extra="forbid")


class ApprovalResolved(BaseModel):
    approval_id: str = Field(min_length=1)
    workflow_id: str = Field(min_length=1)
    step_id: str = Field(min_length=1)
    resolved_by: str = Field(min_length=1)
    approved: bool
    comment: str | None = None

    model_config = ConfigDict(extra="forbid")
