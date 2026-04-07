
from __future__ import annotations
import pytest
pytest.importorskip('playwright')

import pytest

from ikam_perf_report.benchmarks.stagehand_validations import (
    classify_agentic_mismatch,
    parse_agentic_footer,
    validate_debug_pipeline_contract,
)


def test_validate_debug_pipeline_contract_requires_expected_fields() -> None:
    payload = {
        "details": {
            "debug_pipeline": {
                "pipeline_id": "compression-rerender/v1",
                "pipeline_run_id": "pipe-1",
                "pipeline_steps": [
                    "prepare_case",
                    "map",
                    "embed_mapped",
                    "lift",
                    "embed_lifted",
                    "candidate_search",
                    "normalize",
                    "compose_proposal",
                    "verify",
                    "promote_commit",
                    "project_graph",
                ],
                "env_handles": {"dev_env_id": "dev-1", "staging_env_id": "stg-1", "committed_env_id": "main"},
                "step_trace": [{"step_name": "verify", "attempt_index": 2, "retry_parent_step_id": "step-verify-1"}],
                "candidate_discovery": {"tier0_exact_count": 1},
                "verification": {"retry_count": 1},
                "commit_summary": {"ir_unmatched_count": 0},
                "retry_mode": "manual",
                "injection_used": True,
                "retry_reason": "seeded",
            }
        }
    }
    result = validate_debug_pipeline_contract(payload)
    assert result["status"] == "ok"


def test_parse_agentic_footer_extracts_sections() -> None:
    text = """
    I observed retry boundary marker and attempt increment.
    Observed: Retry boundary, A2 badge
    Not Observed: Missing delta intent error
    Uncertain: Cross-env copy status
    """
    parsed = parse_agentic_footer(text)
    assert "Retry boundary" in parsed["observed"]
    assert "Missing delta intent error" in parsed["not_observed"]
    assert "Cross-env copy status" in parsed["uncertain"]


def test_classify_agentic_mismatch_raises_on_contradiction() -> None:
    deterministic = {"retry_boundary": True}
    agentic = {"observed": [], "not_observed": ["retry boundary"], "uncertain": []}
    with pytest.raises(AssertionError):
        classify_agentic_mismatch(deterministic=deterministic, agentic=agentic)
