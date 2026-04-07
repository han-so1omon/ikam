from __future__ import annotations

from dataclasses import dataclass
from typing import Any, List


@dataclass(frozen=True)
class SemanticEntity:
    id: str
    label: str
    kind: str
    payload: dict[str, Any]
    confidence: float
    evidence: List[str]
    referenced_context: List[str]


@dataclass(frozen=True)
class SemanticRelation:
    id: str
    kind: str
    source: str
    target: str
    payload: dict[str, Any]
    confidence: float
    evidence: List[str]
    referenced_context: List[str]
