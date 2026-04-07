from __future__ import annotations

from typing import Any

from ikam.oraculo.reports import EvaluationReport


def serialize_evaluation_report(
    report: EvaluationReport,
    debug_pipeline: dict[str, Any] | None = None,
) -> dict[str, Any]:
    return {
        "report": {
            "compression": {
                "total_fragments": report.compression.total_fragments,
                "unique_fragments": report.compression.unique_fragments,
                "total_bytes": report.compression.total_bytes,
                "unique_bytes": report.compression.unique_bytes,
                "dedup_ratio": report.compression.dedup_ratio,
            },
            "entities": {
                "coverage": report.entities.coverage,
                "passed": report.entities.passed,
            },
            "predicates": {
                "predicate_coverage": report.predicates.predicate_coverage,
                "contradiction_coverage": report.predicates.contradiction_coverage,
                "passed": report.predicates.passed,
            },
            "exploration": {
                "mean_recall": report.exploration.mean_recall,
                "passed": report.exploration.passed,
            },
            "query": {
                "mean_fact_coverage": report.query.mean_fact_coverage,
                "mean_quality_score": report.query.mean_quality_score,
                "passed": report.query.passed,
            },
            "passed": report.passed,
        },
        "details": {
            "pipeline_steps": ["compression", "entities", "predicates", "exploration", "query"],
            "entities": [
                {
                    "expected_name": str(match.expected_name),
                    "found": bool(match.found),
                    "matched_label": match.matched_label,
                }
                for match in report.entities.matches
            ],
            "predicates": [
                {
                    "label": str(match.label),
                    "chain_coverage": float(match.chain_coverage),
                    "matched_hops": list(match.matched_hops),
                }
                for match in report.predicates.matches
            ],
            "contradictions": [
                {
                    "field_name": str(item.field_name),
                    "detected": bool(item.detected),
                    "score": float(item.score),
                }
                for item in report.predicates.contradiction_matches
            ],
            "exploration_queries": [
                {
                    "query": str(result.query),
                    "fragments_retrieved": int(result.fragments_retrieved),
                    "relevance_score": float(result.relevance_score),
                }
                for result in report.exploration.results
            ],
            "query_results": [
                {
                    "query": str(result.query),
                    "facts_found": list(result.facts_found),
                    "fact_coverage": float(result.fact_coverage),
                    "quality_score": float(result.quality_score),
                    "answer_text": str(getattr(result, "answer_text", "")),
                    "evidence_fragment_ids": list(getattr(result, "evidence_fragment_ids", [])),
                }
                for result in report.query.results
            ],
            **({"debug_pipeline": debug_pipeline} if debug_pipeline is not None else {}),
        },
        "rendered": report.render(),
    }
