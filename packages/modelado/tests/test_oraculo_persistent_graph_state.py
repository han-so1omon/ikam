"""Tests for PersistentGraphState — modelado adapter implementing GraphState protocol."""
from __future__ import annotations

import pytest

from ikam.oraculo.graph_state import GraphState


def test_persistent_graph_state_satisfies_protocol():
    """PersistentGraphState must be a runtime-checkable GraphState implementation."""
    from modelado.oraculo.persistent_graph_state import PersistentGraphState

    gs = PersistentGraphState()
    assert isinstance(gs, GraphState)


def test_persistent_graph_state_starts_empty():
    """A fresh PersistentGraphState has no fragments, entities, or relations."""
    from modelado.oraculo.persistent_graph_state import PersistentGraphState

    gs = PersistentGraphState()
    assert gs.fragment_count() == 0
    assert gs.total_bytes() == 0
    assert gs.unique_bytes() == 0
    assert gs.unique_fragment_count() == 0
    assert gs.fragments() == []
    assert gs.entities() == []
    assert gs.relations() == []


def test_persistent_graph_state_add_and_retrieve_fragment():
    """Adding a fragment makes it retrievable by id."""
    from modelado.oraculo.persistent_graph_state import PersistentGraphState
    from ikam.fragments import Fragment

    gs = PersistentGraphState()
    frag = Fragment(cas_id="f1", value="hello world", mime_type="text/plain")
    gs.add_fragment(frag)
    assert gs.fragment_count() == 1
    assert gs.fragment_by_id("f1") is not None
    assert gs.fragment_by_id("f1").value == "hello world"


def test_persistent_graph_state_snapshot_returns_independent_copy():
    """Snapshot must return a copy that doesn't change when the original changes."""
    from modelado.oraculo.persistent_graph_state import PersistentGraphState
    from ikam.fragments import Fragment

    gs = PersistentGraphState()
    gs.add_fragment(Fragment(cas_id="f1", value="before", mime_type="text/plain"))
    snap = gs.snapshot()
    gs.add_fragment(Fragment(cas_id="f2", value="after", mime_type="text/plain"))
    assert gs.fragment_count() == 2
    assert snap.fragment_count() == 1


def test_persistent_graph_state_tracks_total_vs_unique_duplicates():
    from modelado.oraculo.persistent_graph_state import PersistentGraphState
    from ikam.fragments import Fragment

    gs = PersistentGraphState()
    gs.add_fragment(Fragment(cas_id="dup", value="hello", mime_type="text/plain"))
    gs.add_fragment(Fragment(cas_id="dup", value="hello", mime_type="text/plain"))

    assert gs.fragment_count() == 2
    assert gs.unique_fragment_count() == 1
    assert gs.total_bytes() > gs.unique_bytes()


def test_persistent_graph_state_traverse_returns_query_relevant_subset():
    from modelado.oraculo.persistent_graph_state import PersistentGraphState
    from ikam.fragments import Fragment

    gs = PersistentGraphState()
    gs.add_fragment(Fragment(cas_id="a", value="Subscription launch happened in September.", mime_type="text/plain"))
    gs.add_fragment(Fragment(cas_id="b", value="Staffing crunch delayed returns handling.", mime_type="text/plain"))
    gs.add_fragment(Fragment(cas_id="c", value="Packaging vendor switched in June.", mime_type="text/plain"))

    result = gs.traverse("subscription launch", judge=None)

    assert result.fragments
    assert len(result.fragments) < gs.fragment_count()
    assert any(fragment.cas_id == "a" for fragment in result.fragments)
