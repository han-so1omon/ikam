from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime


@dataclass(frozen=True)
class BenchmarkRun:
    id: str
    project_size: str
    dataset_seed: str
    created_at: datetime


@dataclass(frozen=True)
class BenchmarkStage:
    id: str
    run_id: str
    stage_name: str
    started_at: datetime
    ended_at: datetime
    duration_ms: int


@dataclass(frozen=True)
class BenchmarkDecision:
    id: str
    run_id: str
    step_index: int
    decision_type: str
    inputs: dict
    outputs: dict
    created_at: datetime


@dataclass(frozen=True)
class BenchmarkMetric:
    id: str
    run_id: str
    metric_name: str
    metric_value: float
    recorded_at: datetime


@dataclass(frozen=True)
class BenchmarkArtifact:
    id: str
    run_id: str
    artifact_type: str
    artifact_id: str
    created_at: datetime
