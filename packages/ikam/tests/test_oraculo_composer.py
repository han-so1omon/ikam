"""Tests for the Evaluator composer — wires all evaluators together."""
from __future__ import annotations

from ikam.fragments import Fragment
from ikam.forja.contracts import ExtractedEntity, ExtractedRelation, stable_entity_key, stable_relation_key
from ikam.oraculo.graph_state import InMemoryGraphState, MutationRecord
from ikam.oraculo.judge import JudgeQuery, Judgment
from ikam.oraculo.spec import (
    BenchmarkQuery,
    ExpectedContradiction,
    ExpectedEntity,
    ExpectedPredicate,
    OracleSpec,
    Predicate,
)
from ikam.oraculo.composer import Evaluator


class StubJudge:
    def judge(self, query: JudgeQuery) -> Judgment:
        return Judgment(score=0.9, reasoning="stub", facts_found=["Maya Chen"])


def _make_graph() -> InMemoryGraphState:
    gs = InMemoryGraphState()
    gs.add_fragment(Fragment(cas_id="f1", value="Maya founded B&B", mime_type="text/plain"))
    gs.add_entity(ExtractedEntity(
        label="Maya", canonical_label="maya",
        source_fragment_id="f1", entity_key=stable_entity_key("f1", "Maya"),
    ))
    gs.add_relation(ExtractedRelation(
        predicate="founded-by", source_label="B&B", target_label="Maya",
        source_entity_key=stable_entity_key("f1", "B&B"),
        target_entity_key=stable_entity_key("f1", "Maya"),
        relation_key=stable_relation_key("f1", "founded-by", "B&B", "Maya"),
    ))
    return gs


def _make_spec() -> OracleSpec:
    return OracleSpec(
        case_id="test", domain="retail",
        entities=[ExpectedEntity(name="Maya", aliases=[], entity_type="person", source_hint="idea.md")],
        predicates=[ExpectedPredicate(
            label="Maya founded B&B",
            chain=[Predicate(source="Maya", target="B&B", relation_type="founded-by", evidence_hint="deck")],
            inference_type="direct",
        )],
        contradictions=[],
        benchmark_queries=[BenchmarkQuery(
            query="Who founded B&B?", required_facts=["Maya"],
            relevant_artifacts=["idea.md"],
        )],
    )


def test_evaluator_runs_all_quality_dimensions():
    judge = StubJudge()
    gs = _make_graph()
    spec = _make_spec()
    report = Evaluator(judge=judge).evaluate_all(gs, spec)
    assert report.compression.total_fragments >= 0
    assert report.entities.coverage >= 0.0
    assert report.predicates.predicate_coverage >= 0.0
    assert report.exploration.mean_recall >= 0.0
    assert report.query.mean_fact_coverage >= 0.0


def test_evaluator_evaluate_mutation():
    judge = StubJudge()
    gs = _make_graph()
    before = gs.snapshot()
    gs.add_fragment(Fragment(cas_id="f2", value="new data", mime_type="text/plain"))
    mutation = MutationRecord(mutation_id="m1", mutation_type="injection", provenance_recorded=True)
    editing = Evaluator(judge=judge).evaluate_mutation(before, gs, mutation)
    assert editing.provenance_recorded is True
    assert isinstance(editing.passed, bool)


def test_evaluator_evaluate_all_returns_report_with_passed_property():
    judge = StubJudge()
    gs = _make_graph()
    spec = _make_spec()
    report = Evaluator(judge=judge).evaluate_all(gs, spec)
    # passed is a bool derived from sub-reports
    assert isinstance(report.passed, bool)
