from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class BoundarySpan:
    start: int
    end: int
    label: str
    confidence: float = 1.0
    reason: str = ""


@dataclass(frozen=True)
class BoundaryPlan:
    spans: list[BoundarySpan]
    provider: str
    model: str
    prompt_version: str
