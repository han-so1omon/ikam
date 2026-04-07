"""Tests for the six oráculo evaluators — individual evaluator behavior."""
from __future__ import annotations

import pytest

from ikam.fragments import Fragment
from ikam.forja.contracts import ExtractedEntity, ExtractedRelation, stable_entity_key, stable_relation_key
from ikam.oraculo.graph_state import InMemoryGraphState, MutationRecord
from ikam.oraculo.judge import JudgeProtocol, JudgeQuery, Judgment
from ikam.oraculo.spec import (
    BenchmarkQuery,
    ExpectedContradiction,
    ExpectedEntity,
    ExpectedPredicate,
    OracleSpec,
    Predicate,
)


# ------------------------------------------------------------------
# Fixtures
# ------------------------------------------------------------------

class StubJudge:
    """Returns a fixed score for any query."""

    def __init__(self, score: float = 0.9, facts: list[str] | None = None):
        self._score = score
        self._facts = facts or []

    def judge(self, query: JudgeQuery) -> Judgment:
        return Judgment(
            score=self._score,
            reasoning="stub judgment",
            facts_found=self._facts,
        )


def _make_graph_with_entities() -> InMemoryGraphState:
    """Graph with two fragments and two entities."""
    gs = InMemoryGraphState()
    gs.add_fragment(Fragment(cas_id="f1", value="Maya Chen founded Bramble & Bitters", mime_type="text/plain"))
    gs.add_fragment(Fragment(cas_id="f2", value="Revenue is $340K annually", mime_type="text/plain"))
    gs.add_entity(ExtractedEntity(
        label="Maya Chen",
        canonical_label="maya chen",
        source_fragment_id="f1",
        entity_key=stable_entity_key("f1", "Maya Chen"),
    ))
    gs.add_entity(ExtractedEntity(
        label="Bramble & Bitters",
        canonical_label="bramble & bitters",
        source_fragment_id="f1",
        entity_key=stable_entity_key("f1", "Bramble & Bitters"),
    ))
    gs.add_relation(ExtractedRelation(
        predicate="founded-by",
        source_label="Bramble & Bitters",
        target_label="Maya Chen",
        source_entity_key=stable_entity_key("f1", "Bramble & Bitters"),
        target_entity_key=stable_entity_key("f1", "Maya Chen"),
        relation_key=stable_relation_key("f1", "founded-by", "Bramble & Bitters", "Maya Chen"),
    ))
    return gs


def _make_spec() -> OracleSpec:
    return OracleSpec(
        case_id="test",
        domain="retail",
        entities=[
            ExpectedEntity(name="Maya Chen", aliases=["Maya"], entity_type="person", source_hint="idea.md"),
            ExpectedEntity(name="Bramble & Bitters", aliases=["B&B"], entity_type="organization", source_hint="idea.md"),
        ],
        predicates=[
            ExpectedPredicate(
                label="Maya founded B&B",
                chain=[Predicate(source="Maya Chen", target="Bramble & Bitters", relation_type="founded-by", evidence_hint="pitch deck")],
                inference_type="direct",
                confidence_note="strongly stated",
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
            BenchmarkQuery(
                query="Who founded Bramble & Bitters?",
                required_facts=["Maya Chen"],
                relevant_artifacts=["idea.md"],
                expected_contradictions=None,
            ),
        ],
    )


# ------------------------------------------------------------------
# Compression evaluator
# ------------------------------------------------------------------

def test_compression_evaluator_reports_fragment_counts():
    from ikam.oraculo.evaluators.compression import CompressionEvaluator

    gs = _make_graph_with_entities()
    report = CompressionEvaluator().evaluate(gs)
    assert report.total_fragments == 2
    assert report.unique_fragments >= 1
    assert report.total_bytes > 0


# ------------------------------------------------------------------
# Entity evaluator
# ------------------------------------------------------------------

def test_entity_evaluator_finds_matching_entities():
    from ikam.oraculo.evaluators.entities import EntityEvaluator

    gs = _make_graph_with_entities()
    spec = _make_spec()
    report = EntityEvaluator().evaluate(gs, spec, StubJudge())
    assert len(report.matches) == 2
    assert report.coverage > 0.0
    # Both entities exist in graph, so coverage should be 1.0
    assert report.coverage == 1.0
    assert report.passed is True


def test_entity_evaluator_reports_missing_entity():
    from ikam.oraculo.evaluators.entities import EntityEvaluator

    gs = InMemoryGraphState()  # empty graph
    spec = _make_spec()
    report = EntityEvaluator().evaluate(gs, spec, StubJudge(score=0.0))
    assert report.coverage == 0.0
    assert report.passed is False


# ------------------------------------------------------------------
# Predicate evaluator
# ------------------------------------------------------------------

def test_predicate_evaluator_finds_direct_predicate():
    from ikam.oraculo.evaluators.predicates import PredicateEvaluator

    gs = _make_graph_with_entities()
    spec = _make_spec()
    report = PredicateEvaluator().evaluate(gs, spec, StubJudge())
    assert len(report.matches) == 1
    assert report.predicate_coverage > 0.0


def test_predicate_evaluator_reports_contradiction_coverage():
    from ikam.oraculo.evaluators.predicates import PredicateEvaluator

    gs = _make_graph_with_entities()
    spec = _make_spec()
    report = PredicateEvaluator().evaluate(gs, spec, StubJudge())
    assert len(report.contradiction_matches) == 1
    assert isinstance(report.contradiction_coverage, float)


# ------------------------------------------------------------------
# Exploration evaluator
# ------------------------------------------------------------------

def test_exploration_evaluator_retrieves_fragments_for_queries():
    from ikam.oraculo.evaluators.exploration import ExplorationEvaluator

    gs = _make_graph_with_entities()
    spec = _make_spec()
    report = ExplorationEvaluator().evaluate(gs, spec, StubJudge())
    assert len(report.results) == 1  # one benchmark query
    assert report.mean_recall >= 0.0


# ------------------------------------------------------------------
# Editing evaluator
# ------------------------------------------------------------------

def test_editing_evaluator_requires_provenance_and_no_stale_edges():
    from ikam.oraculo.evaluators.editing import EditingEvaluator

    before = _make_graph_with_entities()
    after = before.snapshot()
    after.add_fragment(Fragment(cas_id="f3", value="new content", mime_type="text/plain"))
    mutation = MutationRecord(
        mutation_id="mut1",
        mutation_type="artifact_injection",
        provenance_recorded=True,
    )
    report = EditingEvaluator().evaluate_mutation(before, after, mutation)
    assert report.passed is (report.provenance_recorded and report.cas_integrity and not report.stale_edges)


def test_editing_evaluator_fails_without_provenance():
    from ikam.oraculo.evaluators.editing import EditingEvaluator

    before = _make_graph_with_entities()
    after = before.snapshot()
    mutation = MutationRecord(mutation_id="mut2", mutation_type="test", provenance_recorded=False)
    report = EditingEvaluator().evaluate_mutation(before, after, mutation)
    assert report.passed is False


# ------------------------------------------------------------------
# Query evaluator
# ------------------------------------------------------------------

def test_query_evaluator_checks_fact_coverage():
    from ikam.oraculo.evaluators.query import QueryEvaluator

    gs = _make_graph_with_entities()
    spec = _make_spec()
    judge = StubJudge(score=0.9, facts=["Maya Chen"])
    report = QueryEvaluator().evaluate(gs, spec, judge)
    assert len(report.results) == 1
    assert report.mean_fact_coverage >= 0.0
    assert report.mean_quality_score >= 0.0
