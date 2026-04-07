from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

_VALID_ENV_TYPES = {"dev", "staging", "committed"}
_VALID_STATUSES = {"pending", "running", "succeeded", "failed", "not_executed"}


@dataclass
class DebugRunState:
    run_id: str
    pipeline_id: str
    pipeline_run_id: str
    project_id: str
    operation_id: str
    env_type: str
    env_id: str
    execution_mode: str
    execution_state: str
    current_step_name: str
    current_attempt_index: int
    retry_budget_remaining: int = 3
    resolved_config: dict[str, Any] = field(default_factory=dict)

    def __post_init__(self) -> None:
        if self.env_type not in _VALID_ENV_TYPES:
            raise ValueError(f"Invalid env_type: {self.env_type}")


@dataclass
class DebugStepEvent:
    event_id: str
    run_id: str
    pipeline_id: str
    pipeline_run_id: str
    project_id: str
    operation_id: str
    env_type: str
    env_id: str
    step_name: str
    step_id: str
    status: str
    attempt_index: int
    retry_parent_step_id: str | None
    started_at: str
    ended_at: str | None
    duration_ms: int | None
    metrics: dict[str, Any] = field(default_factory=dict)
    error: dict[str, Any] | None = None

    def __post_init__(self) -> None:
        if self.status not in _VALID_STATUSES:
            raise ValueError(f"Invalid status: {self.status}")
        if self.env_type not in _VALID_ENV_TYPES:
            raise ValueError(f"Invalid env_type: {self.env_type}")
