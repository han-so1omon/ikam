from __future__ import annotations

from ikam.oraculo.evaluators.entities import EntityMatch
from ikam.oraculo.evaluators.exploration import RetrievalResult
from ikam.oraculo.evaluators.predicates import ContradictionMatch, PredicateMatch
from ikam.oraculo.evaluators.query import QueryResult
from ikam.oraculo.reports import (
    CompressionReport,
    EntityReport,
    EvaluationReport,
    ExplorationReport,
    PredicateReport,
    QueryReport,
)
from ikam_perf_report.benchmarks.evaluation_payload import serialize_evaluation_report


def test_serialize_evaluation_report_includes_details():
    report = EvaluationReport(
        compression=CompressionReport(total_fragments=10, unique_fragments=8, total_bytes=1000, unique_bytes=800, dedup_ratio=0.2),
        entities=EntityReport(
            matches=[EntityMatch(expected_name="Maya Chen", found=True, matched_label="maya chen")],
            coverage=0.9,
            passed=True,
        ),
        predicates=PredicateReport(
            matches=[PredicateMatch(label="Maya founded Bramble", chain_coverage=1.0, matched_hops=["Maya-[founded-by]->Bramble"] )],
            contradiction_matches=[ContradictionMatch(field_name="q4_revenue", detected=True, score=0.8)],
            predicate_coverage=0.9,
            contradiction_coverage=1.0,
            passed=True,
        ),
        exploration=ExplorationReport(
            results=[RetrievalResult(query="What launched?", fragments_retrieved=12, relevance_score=0.82)],
            mean_recall=0.82,
            passed=True,
        ),
        query=QueryReport(
            results=[
                QueryResult(
                    query="What launched?",
                    facts_found=["Subscription launched 2025-09"],
                    fact_coverage=1.0,
                    quality_score=8.2,
                    answer_text="The subscription program launched in September 2025.",
                    evidence_fragment_ids=["frag-1", "frag-2"],
                )
            ],
            mean_fact_coverage=1.0,
            mean_quality_score=8.2,
            passed=True,
        ),
    )

    payload = serialize_evaluation_report(report)

    assert "report" in payload
    assert "details" in payload
    assert payload["report"]["compression"]["dedup_ratio"] == 0.2
    assert payload["details"]["entities"][0]["expected_name"] == "Maya Chen"
    assert payload["details"]["predicates"][0]["chain_coverage"] == 1.0
    assert payload["details"]["query_results"][0]["quality_score"] == 8.2
    assert payload["details"]["query_results"][0]["answer_text"]
    assert payload["details"]["query_results"][0]["evidence_fragment_ids"] == ["frag-1", "frag-2"]
