"""Task 9: Evaluation mapping includes debug pipeline payload in details."""
from __future__ import annotations

from unittest.mock import MagicMock

from ikam_perf_report.benchmarks.evaluation_payload import serialize_evaluation_report


def _make_mock_report() -> MagicMock:
    report = MagicMock()
    report.compression.total_fragments = 5
    report.compression.unique_fragments = 4
    report.compression.total_bytes = 1000
    report.compression.unique_bytes = 800
    report.compression.dedup_ratio = 0.2
    report.entities.coverage = 0.9
    report.entities.passed = True
    report.entities.matches = []
    report.predicates.predicate_coverage = 0.85
    report.predicates.contradiction_coverage = 1.0
    report.predicates.passed = True
    report.predicates.matches = []
    report.predicates.contradiction_matches = []
    report.exploration.mean_recall = 0.8
    report.exploration.passed = True
    report.exploration.results = []
    report.query.mean_fact_coverage = 0.75
    report.query.mean_quality_score = 8.0
    report.query.passed = True
    report.query.results = []
    report.passed = True
    report.render.return_value = "ok"
    return report


def test_serialize_without_debug_pipeline_omits_key():
    """Without debug_pipeline argument, details must not include that key."""
    payload = serialize_evaluation_report(_make_mock_report())
    assert "debug_pipeline" not in payload["details"]


def test_serialize_with_debug_pipeline_includes_section():
    """When debug_pipeline is provided, details.debug_pipeline must include
    pipeline_id, pipeline_run_id, step_trace, and env_handles."""
    debug_pipeline = {
        "pipeline_id": "compression-rerender/v1",
        "pipeline_run_id": "run-abc123",
        "step_trace": ["prepare_case", "map", "embed_mapped"],
        "env_handles": [{"env_type": "dev", "env_id": "dev-1"}],
    }
    payload = serialize_evaluation_report(_make_mock_report(), debug_pipeline=debug_pipeline)
    dp = payload["details"]["debug_pipeline"]
    assert dp["pipeline_id"] == "compression-rerender/v1"
    assert dp["pipeline_run_id"] == "run-abc123"
    assert dp["step_trace"] == ["prepare_case", "map", "embed_mapped"]
    assert dp["env_handles"] == [{"env_type": "dev", "env_id": "dev-1"}]


def test_serialize_with_partial_debug_pipeline():
    """Partial debug_pipeline dict (only pipeline_id/run_id, no step_trace)
    must still be passed through without KeyError."""
    debug_pipeline = {
        "pipeline_id": "compression-rerender/v1",
        "pipeline_run_id": "run-xyz",
    }
    payload = serialize_evaluation_report(_make_mock_report(), debug_pipeline=debug_pipeline)
    dp = payload["details"]["debug_pipeline"]
    assert dp["pipeline_id"] == "compression-rerender/v1"
    assert dp["pipeline_run_id"] == "run-xyz"
    # step_trace and env_handles absent from input → absent from output
    assert "step_trace" not in dp
    assert "env_handles" not in dp
