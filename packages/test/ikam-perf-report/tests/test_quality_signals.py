from __future__ import annotations

from ikam_perf_report.benchmarks.quality_signals import (
    build_commit_receipt,
    build_query_evaluations,
    compute_lane_metrics,
    evaluate_commit_lane_gates,
)


def test_build_query_evaluations_emits_three_priority_queries():
    evaluations = build_query_evaluations(
        "case-a",
        graph_nodes=500,
        graph_edges=220,
        asset_mime_types=["text/markdown", "application/pdf", "application/json"],
        normalized_fragments=180,
        entity_fragments=90,
        relation_fragments=45,
        promoted_ratio=0.82,
    )

    assert [item["query_id"] for item in evaluations] == [
        "case-a:business-identity",
        "case-a:storage-gains",
        "case-a:reliability",
    ]


def test_build_query_evaluations_varies_oracle_scores_with_inputs():
    smaller = build_query_evaluations(
        "case-small",
        graph_nodes=100,
        graph_edges=20,
        asset_mime_types=["text/markdown"],
        normalized_fragments=80,
        entity_fragments=10,
        relation_fragments=5,
        promoted_ratio=0.6,
    )
    larger = build_query_evaluations(
        "case-large",
        graph_nodes=900,
        graph_edges=480,
        asset_mime_types=["text/markdown", "application/pdf", "application/json", "application/vnd.ms-excel"],
        normalized_fragments=250,
        entity_fragments=180,
        relation_fragments=120,
        promoted_ratio=0.95,
    )

    small_scores = [(item["oracle"]["coverage"], item["oracle"]["grounded_precision"]) for item in smaller]
    large_scores = [(item["oracle"]["coverage"], item["oracle"]["grounded_precision"]) for item in larger]
    assert small_scores != large_scores


def test_lane_metrics_expose_expected_quality_surfaces() -> None:
    metrics = compute_lane_metrics(
        normalized_fragments=120,
        entity_fragments=60,
        relation_fragments=30,
        graph_edges=45,
        cas_hit_rate=0.8,
        second_promoted=0,
    )
    assert set(metrics) >= {
        "exploration_variability",
        "relation_commit_yield",
        "evidence_grounding_ratio",
        "edge_idempotency_integrity",
        "within_case_reuse_primary",
        "cross_case_reuse_secondary",
    }
    assert metrics["within_case_reuse_primary"] >= metrics["cross_case_reuse_secondary"]


def test_commit_lane_gate_evaluation_fails_on_low_quality() -> None:
    result = evaluate_commit_lane_gates(
        {
            "evidence_grounding_ratio": 0.05,
            "evidence_coverage_ratio": 0.05,
            "endpoint_integrity": 0.5,
            "edge_idempotency_integrity": 0.0,
        }
    )
    assert result["passed"] is False
    assert set(result["failures"]) == {
        "grounded_precision_floor",
        "evidence_coverage_floor",
        "endpoint_integrity_floor",
        "replay_idempotency_floor",
    }


def test_commit_receipt_is_deterministic() -> None:
    first = build_commit_receipt(
        case_id="case-a",
        mode="commit-strict",
        committed_fragment_ids=["f2", "f1"],
        edge_idempotency_keys=["e2", "e1"],
        unresolved_endpoints=[],
    )
    second = build_commit_receipt(
        case_id="case-a",
        mode="commit-strict",
        committed_fragment_ids=["f1", "f2"],
        edge_idempotency_keys=["e1", "e2"],
        unresolved_endpoints=[],
    )
    assert first["receipt_id"] == second["receipt_id"]
