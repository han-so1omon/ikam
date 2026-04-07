"""Tests for GraphState protocol and InMemoryGraphState."""
from __future__ import annotations

from ikam.oraculo.graph_state import GraphState, InMemoryGraphState
from ikam.fragments import Fragment


def test_in_memory_graph_state_implements_protocol():
    gs = InMemoryGraphState()
    assert isinstance(gs, GraphState)


def test_in_memory_graph_state_fragments_empty():
    gs = InMemoryGraphState()
    assert gs.fragments() == []
    assert gs.fragment_count() == 0


def test_in_memory_graph_state_add_and_retrieve_fragment():
    gs = InMemoryGraphState()
    frag = Fragment(cas_id="abc123", value="hello world", mime_type="text/plain")
    gs.add_fragment(frag)
    assert gs.fragment_count() == 1
    assert gs.fragment_by_id("abc123") is frag


def test_in_memory_graph_state_entities_empty():
    gs = InMemoryGraphState()
    assert gs.entities() == []


def test_in_memory_graph_state_total_bytes():
    gs = InMemoryGraphState()
    frag = Fragment(cas_id="x", value="hello", mime_type="text/plain")
    gs.add_fragment(frag)
    assert gs.total_bytes() > 0


def test_in_memory_graph_state_snapshot_is_independent_copy():
    gs = InMemoryGraphState()
    frag = Fragment(cas_id="x", value="hello", mime_type="text/plain")
    gs.add_fragment(frag)
    snap = gs.snapshot()
    assert snap.fragment_count() == 1
    # Adding to original doesn't affect snapshot
    gs.add_fragment(Fragment(cas_id="y", value="world", mime_type="text/plain"))
    assert snap.fragment_count() == 1
    assert gs.fragment_count() == 2


def test_in_memory_graph_state_tracks_total_vs_unique_for_duplicate_fragments():
    gs = InMemoryGraphState()
    gs.add_fragment(Fragment(cas_id="dup", value="same", mime_type="text/plain"))
    gs.add_fragment(Fragment(cas_id="dup", value="same", mime_type="text/plain"))

    assert gs.fragment_count() == 2
    assert gs.unique_fragment_count() == 1
    assert gs.total_bytes() > gs.unique_bytes()


def test_in_memory_graph_state_traverse_returns_ranked_subset_not_all_fragments():
    gs = InMemoryGraphState()
    gs.add_fragment(Fragment(cas_id="a", value="Subscription launch in September 2025", mime_type="text/plain"))
    gs.add_fragment(Fragment(cas_id="b", value="Holiday staffing crunch impacted returns", mime_type="text/plain"))
    gs.add_fragment(Fragment(cas_id="c", value="Vendor onboarding process", mime_type="text/plain"))

    result = gs.traverse("subscription launch", judge=None)

    assert result.fragments
    assert len(result.fragments) < gs.fragment_count()
    ids = [f.cas_id for f in result.fragments]
    assert "a" in ids
