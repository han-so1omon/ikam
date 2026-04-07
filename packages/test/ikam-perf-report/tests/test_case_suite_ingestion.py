from __future__ import annotations

import pytest

from ikam_perf_report.benchmarks.case_fixtures import available_case_ids, load_case_fixture
from ikam_perf_report.benchmarks.case_suite_ingestion import run_single_case_ingestion


def _existing_case_ids() -> list[str]:
    try:
        case_ids = available_case_ids()
    except FileNotFoundError:
        return []
    existing: list[str] = []
    for case_id in case_ids:
        try:
            fixture = load_case_fixture(case_id)
        except FileNotFoundError:
            continue
        if fixture.assets:
            existing.append(case_id)
    return existing


@pytest.mark.parametrize("case_id", _existing_case_ids())
def test_run_single_case_ingestion(case_id: str):
    result = run_single_case_ingestion(case_id)

    assert result["case_id"] == case_id
    assert result["asset_count"] >= 1
    assert result["ingestion_size_bytes"] >= 1

    assert result["staging"]["rows"] >= 1
    assert result["promotion"]["permanent_rows"] == result["staging"]["rows"]

    assert result["graph"]["nodes"] >= 1
    assert "graph TD" in result["graph"]["mermaid"]

    assert result["processes"]["source_fragments"] >= 1
    assert result["processes"]["normalized_fragments"] >= 1
    assert result["processes"]["enriched_fragments"] >= 1

    assert 0.0 <= result["dedup"]["cas_hit_rate"] <= 1.0

    aqs = result["answer_quality"]
    assert aqs["aqs"] >= 0.0
    assert aqs["aqs"] <= 1.0
    assert aqs["review_mode"] in {"manual", "oracle-defaulted"}
    assert isinstance(aqs["query_scores"], list)
    assert len(aqs["query_scores"]) >= 1
    first_query = aqs["query_scores"][0]
    assert "oracle" in first_query
    assert "review" in first_query
    assert "oracle_score" in first_query
    assert "reviewer_score" in first_query
    assert "aqs" in first_query

    lane_metrics = result["lane_metrics"]
    assert lane_metrics["within_case_reuse_primary"] >= lane_metrics["cross_case_reuse_secondary"]
    assert lane_metrics["edge_idempotency_integrity"] in {0.0, 1.0}

    commit_gates = result["commit_lane_gates"]
    assert "passed" in commit_gates
    assert "thresholds" in commit_gates
    assert "failures" in commit_gates
