"""Tests for mutation suggestion from spec-vs-graph gap analysis."""
from __future__ import annotations

from ikam.fragments import Fragment
from ikam.forja.contracts import ExtractedEntity, ExtractedRelation, stable_entity_key, stable_relation_key
from ikam.oraculo.graph_state import InMemoryGraphState
from ikam.oraculo.spec import (
    BenchmarkQuery,
    ExpectedContradiction,
    ExpectedEntity,
    ExpectedPredicate,
    OracleSpec,
    Predicate,
)


def _make_spec_with_gaps() -> OracleSpec:
    """Spec that expects more than the graph provides, creating gaps."""
    return OracleSpec(
        case_id="test",
        domain="retail",
        entities=[
            ExpectedEntity(name="Maya Chen", aliases=["Maya"], entity_type="person", source_hint="idea.md"),
            ExpectedEntity(name="Bramble & Bitters", aliases=["B&B"], entity_type="organization", source_hint="idea.md"),
            ExpectedEntity(name="Missing Entity", aliases=[], entity_type="person", source_hint="unknown.md"),
        ],
        predicates=[
            ExpectedPredicate(
                label="Maya founded B&B",
                chain=[Predicate(source="Maya Chen", target="Bramble & Bitters", relation_type="founded-by", evidence_hint="deck")],
                inference_type="direct",
            ),
        ],
        contradictions=[
            ExpectedContradiction(
                field="revenue",
                conflicting_values=["$340K", "$410K"],
                artifacts_involved=["pitch_deck.pptx", "financials.xlsx"],
                resolution_hint=None,
            ),
        ],
        benchmark_queries=[
            BenchmarkQuery(query="Who founded B&B?", required_facts=["Maya Chen"], relevant_artifacts=["idea.md"]),
        ],
    )


def _make_partial_graph() -> InMemoryGraphState:
    """Graph with some but not all expected data — creates gaps for mutations."""
    gs = InMemoryGraphState()
    gs.add_fragment(Fragment(cas_id="f1", value="Maya Chen founded B&B", mime_type="text/plain"))
    gs.add_entity(ExtractedEntity(
        label="Maya Chen", canonical_label="maya chen",
        source_fragment_id="f1", entity_key=stable_entity_key("f1", "Maya Chen"),
    ))
    # Bramble & Bitters entity is present
    gs.add_entity(ExtractedEntity(
        label="Bramble & Bitters", canonical_label="bramble & bitters",
        source_fragment_id="f1", entity_key=stable_entity_key("f1", "Bramble & Bitters"),
    ))
    # "Missing Entity" is NOT in the graph — this creates a gap
    return gs


def test_suggest_mutations_returns_proposals_for_gaps():
    from ikam.oraculo.mutations import suggest_mutations

    spec = _make_spec_with_gaps()
    graph = _make_partial_graph()
    proposals = suggest_mutations(spec, graph)
    assert isinstance(proposals, list)
    assert len(proposals) >= 1


def test_suggest_mutations_returns_contradiction_resolution_candidates():
    from ikam.oraculo.mutations import suggest_mutations

    spec = _make_spec_with_gaps()
    graph = _make_partial_graph()
    proposals = suggest_mutations(spec, graph)
    assert any(p.mutation_type == "contradiction_resolution" for p in proposals)


def test_suggest_mutations_returns_entity_correction_for_missing():
    from ikam.oraculo.mutations import suggest_mutations

    spec = _make_spec_with_gaps()
    graph = _make_partial_graph()
    proposals = suggest_mutations(spec, graph)
    assert any(p.mutation_type == "entity_correction" for p in proposals)
