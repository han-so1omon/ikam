from modelado.knowledge_edge_events import (
    canonicalize_predicate,
    iter_relation_pairs,
    build_knowledge_edge_events,
)


def test_canonicalize_predicate_stable():
    assert canonicalize_predicate(" depends on ") == "depends_on"
    assert canonicalize_predicate("Depends-On") == "depends_on"
    assert canonicalize_predicate("  ") == "relation"


def test_iter_relation_pairs_directed_dedup_sorted():
    pairs = iter_relation_pairs(
        subject_ids=["b", "a", "a"],
        object_ids=["d", "c"],
        directed=True,
    )
    assert pairs == [("a", "c"), ("a", "d"), ("b", "c"), ("b", "d")]


def test_iter_relation_pairs_undirected_emits_both_directions():
    pairs = iter_relation_pairs(
        subject_ids=["a"],
        object_ids=["c"],
        directed=False,
    )
    assert pairs == [("a", "c"), ("c", "a")]


def test_build_knowledge_edge_events_deterministic_and_unique():
    events1 = build_knowledge_edge_events(
        op="upsert",
        relation_fragment_id="rf-1",
        predicate="Depends On",
        subject_fragment_ids=["B", "A", "A"],
        object_fragment_ids=["D", "C"],
        directed=True,
        confidence_score=0.9,
        qualifiers={"source": "llm_extraction"},
    )
    events2 = build_knowledge_edge_events(
        op="upsert",
        relation_fragment_id="rf-1",
        predicate="Depends On",
        subject_fragment_ids=["A", "B"],
        object_fragment_ids=["C", "D"],
        directed=True,
        confidence_score=0.9,
        qualifiers={"source": "llm_extraction"},
    )

    assert [e.idempotency_key for e in events1] == [e.idempotency_key for e in events2]
    assert [e.edge_identity_key for e in events1] == [e.edge_identity_key for e in events2]
    assert {e.idempotency_key for e in events1} == set([e.idempotency_key for e in events1])
    assert {e.edge_identity_key for e in events1} == set([e.edge_identity_key for e in events1])
    assert all(e.edge_label == "knowledge:depends_on" for e in events1)
