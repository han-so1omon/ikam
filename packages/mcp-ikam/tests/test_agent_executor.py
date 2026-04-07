from __future__ import annotations

import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "src"
if str(SRC) not in sys.path:
    sys.path.insert(0, str(SRC))


def test_run_parse_review_returns_executor_identity_and_agent_spec() -> None:
    from mcp_ikam.agent_executor import run_parse_review

    result = run_parse_review(
        {
            "agent_spec": {"logical_name": "parse-review-agent"},
            "map": {"root_node_id": "map:root", "node_count": 1},
        }
    )

    assert result["executor_id"] == "executor://agent-env-primary"
    assert result["agent_spec"]["logical_name"] == "parse-review-agent"
    assert result["decision"] in {"accept", "warn", "reject"}


def test_parse_review_approval_mode_defaults_to_human_required(monkeypatch) -> None:
    from mcp_ikam.agent_executor import resolve_parse_review_approval_mode

    monkeypatch.delenv("IKAM_PARSE_REVIEW_APPROVAL_MODE", raising=False)

    assert resolve_parse_review_approval_mode() == "human_required"


def test_run_parse_review_builds_interacciones_elicitation(monkeypatch) -> None:
    from mcp_ikam.agent_executor import run_parse_review

    monkeypatch.setenv("IKAM_PARSE_REVIEW_APPROVAL_MODE", "human_required")

    result = run_parse_review(
        {
            "workflow_id": "wf-parse-1",
            "step_id": "map.conceptual.lift.surface_fragments",
            "agent_spec": {
                "logical_name": "parse-review-agent",
                "artifact_ref": {"fragment_id": "agent-spec:parse-review-agent", "head_fragment_id": "cas://agent-spec"},
            },
            "map": {"root_node_id": "map:root", "node_count": 0, "relationship_count": 0, "segment_candidate_count": 0},
            "trace_events": [{"phase": "llm_plan_returned", "message": "response received"}],
            "artifacts": [{"artifact_id": "artifact://a"}],
            "generation_provenance": {"provider": "openai", "model": "gpt-4o-mini"},
        }
    )

    elicitation = result["elicitation"]
    assert elicitation["approval_mode"] == "human_required"
    assert elicitation["approval_request"]["requested_by"] == "executor://agent-env-primary"
    assert elicitation["approval_request"]["details"]["kind"] == "parse_review_elicitation"
    assert elicitation["approval_request"]["details"]["judgment"]["decision"] == result["decision"]


def test_run_parse_review_auto_approve_emits_resolution(monkeypatch) -> None:
    from mcp_ikam.agent_executor import run_parse_review

    monkeypatch.setenv("IKAM_PARSE_REVIEW_APPROVAL_MODE", "auto_approve")

    result = run_parse_review(
        {
            "workflow_id": "wf-parse-2",
            "step_id": "map.conceptual.lift.surface_fragments",
            "agent_spec": {"logical_name": "parse-review-agent"},
            "map": {"root_node_id": "map:root", "node_count": 1, "relationship_count": 0, "segment_candidate_count": 1},
        }
    )

    assert result["elicitation"]["approval_mode"] == "auto_approve"
    assert result["elicitation"]["approval_resolution"]["approved"] is True
