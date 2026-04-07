"""Conditional routing for graph edges."""
from __future__ import annotations

from typing import Any, Callable, Dict, Mapping


ConditionFn = Callable[[Mapping[str, Any]], str]


class EdgeCondition:
    """Condition-based edge routing.

    Example:
        def route_by_confidence(state):
            return "high" if state.get("confidence", 0) > 0.8 else "low"

        cond = EdgeCondition(route_by_confidence, {"high": "auto", "low": "human"})
        next_id = cond.select({"confidence": 0.9})  # -> "auto"
    """

    def __init__(self, condition: ConditionFn, targets: Dict[str, str]) -> None:
        self.condition = condition
        self.targets = dict(targets)

    def select(self, state: Mapping[str, Any]) -> str:
        key = self.condition(state)
        if key not in self.targets:
            raise KeyError(f"Condition returned '{key}', but no target configured. Known: {list(self.targets)}")
        return self.targets[key]
