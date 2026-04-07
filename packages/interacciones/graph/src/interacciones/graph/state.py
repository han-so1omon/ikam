"""State merging with explicit reducers.

Provides reducer functions and a merge utility that applies per-key reducers.
Defaults to replace semantics for keys without a configured reducer.
"""
from __future__ import annotations

from typing import Any, Callable, Dict, Mapping

Reducer = Callable[[Any, Any], Any]


def replace_reducer(old: Any, new: Any) -> Any:
    """Replace old value with new value (default behavior)."""
    return new


def append_reducer(old: Any, new: Any) -> Any:
    """Append list-like values: returns (old or []) + (new or [])."""
    old_list = list(old or [])
    new_list = list(new or [])
    return old_list + new_list


def update_reducer(old: Any, new: Any) -> Any:
    """Update dict-like values: shallow merge old with new (new wins)."""
    old_dict = dict(old or {})
    new_dict = dict(new or {})
    merged = {**old_dict, **new_dict}
    return merged


class GraphState:
    """Graph state wrapper with per-key reducer semantics.

    Example:
        reducers = {
            "messages": append_reducer,
            "context": update_reducer,
        }
        state = GraphState(reducers)
        merged = state.merge({"messages": [1]}, {"messages": [2], "count": 1})
        # merged == {"messages": [1,2], "count": 1}
    """

    def __init__(self, reducers: Mapping[str, Reducer] | None = None) -> None:
        self._reducers: Dict[str, Reducer] = dict(reducers or {})

    def get_reducer(self, key: str) -> Reducer:
        return self._reducers.get(key, replace_reducer)

    def merge(self, base: Mapping[str, Any], delta: Mapping[str, Any]) -> Dict[str, Any]:
        """Merge delta into base using reducers per key.

        - For keys only in base, keep base value
        - For keys in delta, apply reducer(key)(base_value, delta_value)
        """
        result: Dict[str, Any] = dict(base)
        for key, new_val in delta.items():
            old_val = result.get(key)
            reducer = self.get_reducer(key)
            result[key] = reducer(old_val, new_val)
        return result

    def with_reducer(self, key: str, reducer: Reducer) -> "GraphState":
        """Return a new GraphState with a different reducer for a specific key."""
        new_map = dict(self._reducers)
        new_map[key] = reducer
        return GraphState(new_map)
