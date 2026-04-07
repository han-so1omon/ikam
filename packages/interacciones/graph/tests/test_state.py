"""Tests for state reducers and merge behavior."""
from interacciones.graph import GraphState, append_reducer, replace_reducer, update_reducer


def test_replace_reducer():
    assert replace_reducer(1, 2) == 2
    assert replace_reducer(None, {"a": 1}) == {"a": 1}


def test_append_reducer():
    assert append_reducer([1], [2, 3]) == [1, 2, 3]
    assert append_reducer(None, [1]) == [1]
    assert append_reducer([1], None) == [1]


def test_update_reducer():
    assert update_reducer({"a": 1}, {"b": 2}) == {"a": 1, "b": 2}
    assert update_reducer(None, {"a": 1}) == {"a": 1}


def test_graph_state_merge_defaults_to_replace():
    gs = GraphState()
    base = {"a": 1}
    delta = {"a": 2, "b": 3}
    merged = gs.merge(base, delta)
    assert merged == {"a": 2, "b": 3}


def test_graph_state_merge_with_custom_reducers():
    gs = GraphState({"messages": append_reducer, "ctx": update_reducer})
    base = {"messages": [1], "ctx": {"a": 1}, "count": 1}
    delta = {"messages": [2], "ctx": {"b": 2}, "count": 2}
    merged = gs.merge(base, delta)
    assert merged["messages"] == [1, 2]
    assert merged["ctx"] == {"a": 1, "b": 2}
    assert merged["count"] == 2
