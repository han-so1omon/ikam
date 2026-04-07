from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any, Dict, List


@dataclass
class DecisionRecord:
    step_index: int
    decision_type: str
    inputs: Dict[str, Any]
    outputs: Dict[str, Any]
    created_at: datetime


class DecisionTraceCollector:
    def __init__(self) -> None:
        self._records: List[DecisionRecord] = []

    def record(self, step_index: int, decision_type: str, inputs: Dict[str, Any], outputs: Dict[str, Any]) -> None:
        self._records.append(
            DecisionRecord(
                step_index=step_index,
                decision_type=decision_type,
                inputs=inputs,
                outputs=outputs,
                created_at=datetime.now(timezone.utc),
            )
        )

    def to_dicts(self) -> List[Dict[str, Any]]:
        return [
            {
                "step_index": record.step_index,
                "decision_type": record.decision_type,
                "inputs": record.inputs,
                "outputs": record.outputs,
                "created_at": record.created_at.isoformat(),
            }
            for record in self._records
        ]
