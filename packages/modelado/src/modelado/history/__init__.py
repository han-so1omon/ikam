from __future__ import annotations

import datetime as dt
from dataclasses import dataclass, field
from typing import Any, Dict, List, Literal


@dataclass(frozen=True)
class VerificationResult:
    run_id: str
    transition_id: str
    status: Literal["success", "failed"]
    drift_score: float
    metadata: Dict[str, Any] = field(default_factory=dict)
    ts: dt.datetime = field(default_factory=dt.datetime.now)


class HistoricalFeedback:
    """In-memory historical feedback store for ingestion verification outcomes."""

    def __init__(self):
        self.history: List[VerificationResult] = []
        self.pattern_scores: Dict[str, List[float]] = {}

    def record_result(self, result: VerificationResult):
        self.history.append(result)
        pattern_id = result.metadata.get("pattern_id")
        if pattern_id:
            self.pattern_scores.setdefault(pattern_id, []).append(1.0 if result.status == "success" else 0.0)

    def get_pattern_reliability(self, pattern_id: str) -> float:
        scores = self.pattern_scores.get(pattern_id)
        if not scores:
            return 0.5
        return sum(scores) / len(scores)

    def get_recommendations(self, dna_fingerprint: str) -> List[Dict[str, Any]]:
        return [
            {
                "pattern_id": "pattern_std_paragraph_v1",
                "reliability": self.get_pattern_reliability("pattern_std_paragraph_v1"),
                "sample_drift": 0.001,
            }
        ]
