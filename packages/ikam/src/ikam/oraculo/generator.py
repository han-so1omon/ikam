"""Oracle spec generation — builds OracleSpec from idea text via judge queries."""
from __future__ import annotations

from ikam.oraculo.judge import JudgeProtocol, JudgeQuery
from ikam.oraculo.spec import (
    BenchmarkQuery,
    ExpectedContradiction,
    ExpectedEntity,
    ExpectedPredicate,
    OracleSpec,
    Predicate,
)


def generate_oracle_spec(
    *,
    idea_text: str,
    case_id: str,
    judge: JudgeProtocol,
) -> OracleSpec:
    """Generate an OracleSpec by querying the judge for entities, predicates, contradictions, and queries."""
    entities = _extract_entities(idea_text, judge)
    predicates = _extract_predicates(idea_text, judge)
    contradictions = _extract_contradictions(idea_text, judge)
    benchmark_queries = _extract_benchmark_queries(idea_text, judge)

    return OracleSpec(
        case_id=case_id,
        domain="general",
        entities=entities,
        predicates=predicates,
        contradictions=contradictions,
        benchmark_queries=benchmark_queries,
    )


def _extract_entities(idea_text: str, judge: JudgeProtocol) -> list[ExpectedEntity]:
    judgment = judge.judge(JudgeQuery(
        question="Extract all entities (people, organizations, products, places) from this text.",
        context={"text": idea_text, "task": "entities"},
    ))
    raw = judgment.metadata.get("entities", [])
    return [
        ExpectedEntity(
            name=e.get("name", ""),
            aliases=e.get("aliases", []),
            entity_type=e.get("entity_type", "unknown"),
            source_hint=e.get("source_hint", "idea text"),
        )
        for e in raw
        if e.get("name")
    ]


def _extract_predicates(idea_text: str, judge: JudgeProtocol) -> list[ExpectedPredicate]:
    judgment = judge.judge(JudgeQuery(
        question="Extract all relationships/predicates between entities in this text.",
        context={"text": idea_text, "task": "predicates"},
    ))
    raw = judgment.metadata.get("predicates", [])
    return [
        ExpectedPredicate(
            label=p.get("label", ""),
            chain=[
                Predicate(
                    source=c.get("source", ""),
                    target=c.get("target", ""),
                    relation_type=c.get("relation_type", ""),
                    evidence_hint=c.get("evidence_hint", ""),
                )
                for c in p.get("chain", [])
            ],
            inference_type=p.get("inference_type", "direct"),
            confidence_note=p.get("confidence_note"),
        )
        for p in raw
        if p.get("label")
    ]


def _extract_contradictions(idea_text: str, judge: JudgeProtocol) -> list[ExpectedContradiction]:
    judgment = judge.judge(JudgeQuery(
        question="Identify potential contradictions or conflicting data in this text.",
        context={"text": idea_text, "task": "contradictions"},
    ))
    raw = judgment.metadata.get("contradictions", [])
    return [
        ExpectedContradiction(
            field=c.get("field", ""),
            conflicting_values=c.get("conflicting_values", []),
            artifacts_involved=c.get("artifacts_involved", []),
            resolution_hint=c.get("resolution_hint"),
        )
        for c in raw
        if c.get("field")
    ]


def _extract_benchmark_queries(idea_text: str, judge: JudgeProtocol) -> list[BenchmarkQuery]:
    judgment = judge.judge(JudgeQuery(
        question="Generate benchmark queries to test understanding of this text.",
        context={"text": idea_text, "task": "benchmark_queries"},
    ))
    raw = judgment.metadata.get("benchmark_queries", [])
    return [
        BenchmarkQuery(
            query=q.get("query", ""),
            required_facts=q.get("required_facts", []),
            relevant_artifacts=q.get("relevant_artifacts", []),
            expected_contradictions=q.get("expected_contradictions"),
        )
        for q in raw
        if q.get("query")
    ]
