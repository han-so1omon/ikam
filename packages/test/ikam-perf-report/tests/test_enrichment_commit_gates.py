from __future__ import annotations

from ikam_perf_report.benchmarks.quality_signals import evaluate_commit_lane_gates


def test_commit_lane_gates_pass_for_valid_commit_metrics() -> None:
    result = evaluate_commit_lane_gates(
        {
            "evidence_grounding_ratio": 0.9,
            "evidence_coverage_ratio": 0.9,
            "endpoint_integrity": 1.0,
            "edge_idempotency_integrity": 1.0,
        }
    )
    assert result["passed"] is True
    assert result["failures"] == []


def test_commit_lane_gates_fail_when_provenance_or_evidence_missing() -> None:
    result = evaluate_commit_lane_gates(
        {
            "evidence_grounding_ratio": 0.0,
            "evidence_coverage_ratio": 0.0,
            "endpoint_integrity": 0.0,
            "edge_idempotency_integrity": 0.0,
        }
    )
    assert result["passed"] is False
    assert len(result["failures"]) == 4
