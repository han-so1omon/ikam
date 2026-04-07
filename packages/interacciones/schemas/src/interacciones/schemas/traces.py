"""Canonical trace persistence contracts for orchestration."""

from __future__ import annotations

from enum import Enum

from pydantic import BaseModel, ConfigDict


class TracePersistenceMode(str, Enum):
    NONE = "none"
    ON_FAILURE = "on_failure"
    ON_APPROVAL = "on_approval"
    PER_STEP = "per_step"
    BATCH = "batch"
    FINAL_ONLY = "final_only"


class TracePersistencePolicy(BaseModel):
    mode: TracePersistenceMode

    model_config = ConfigDict(extra="forbid")
