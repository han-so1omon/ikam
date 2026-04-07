from __future__ import annotations

import os
from typing import Any
from uuid import uuid4

from interacciones.schemas.execution import ApprovalRequested, ApprovalResolved


_DEFAULT_AGENT_SPEC = {
    "logical_name": "parse-review-agent",
    "artifact_ref": {
        "fragment_id": "agent-spec:parse-review-agent",
        "head_fragment_id": "cas://agent-spec/parse-review-agent",
    },
}


def resolve_parse_review_approval_mode() -> str:
    mode = os.getenv("IKAM_PARSE_REVIEW_APPROVAL_MODE", "human_required").strip().lower()
    if mode not in {"auto_approve", "human_required", "agent_required"}:
        return "human_required"
    return mode


def _agent_spec(payload: dict[str, Any]) -> dict[str, Any]:
    agent_spec = payload.get("agent_spec")
    if isinstance(agent_spec, dict):
        return {**_DEFAULT_AGENT_SPEC, **agent_spec}
    return dict(_DEFAULT_AGENT_SPEC)


def run_parse_review(payload: dict[str, Any]) -> dict[str, Any]:
    agent_spec = _agent_spec(payload)
    map_payload = payload.get("map") if isinstance(payload.get("map"), dict) else {}
    generation_provenance = payload.get("generation_provenance") if isinstance(payload.get("generation_provenance"), dict) else {}
    artifacts = payload.get("artifacts") if isinstance(payload.get("artifacts"), list) else []
    trace_events = payload.get("trace_events") if isinstance(payload.get("trace_events"), list) else []
    node_count = int(map_payload.get("node_count") or 0)
    relationship_count = int(map_payload.get("relationship_count") or 0)
    segment_candidate_count = int(map_payload.get("segment_candidate_count") or 0)
    decision = "accept" if node_count > 0 else "reject"
    findings = [
        "semantic map contains nodes" if decision == "accept" else "semantic map is empty"
    ]
    suggested_repairs = [] if decision == "accept" else ["rerun map generation with revised grouping"]
    confidence = 0.84 if decision == "accept" else 0.31
    summary = (
        "Parse review approved semantic map; confirmation required"
        if decision == "accept"
        else "Parse review rejected semantic map for human approval"
    )
    approval_mode = resolve_parse_review_approval_mode()
    details = {
        "kind": "parse_review_elicitation",
        "operation_name": payload.get("step_id") or "map.conceptual.lift.surface_fragments",
        "approval_mode": approval_mode,
        "agent": {
            "executor_id": "executor://agent-env-primary",
            "logical_name": agent_spec["logical_name"],
            "artifact_ref": agent_spec.get("artifact_ref") or {},
        },
        "judgment": {
            "decision": decision,
            "confidence": confidence,
            "findings": findings,
            "suggested_repairs": suggested_repairs,
        },
        "mcp": {
            "method": "generate_structural_map",
            "provider": str(generation_provenance.get("provider") or "unknown"),
            "model": str(generation_provenance.get("model") or "unknown"),
            "trace_summary": {
                "event_count": len(trace_events),
                "last_phase": str(trace_events[-1].get("phase") or "") if trace_events and isinstance(trace_events[-1], dict) else "",
            },
        },
        "artifacts": {
            "artifact_count": len(artifacts),
            "sample_artifact_ids": [str(item.get("artifact_id")) for item in artifacts[:2] if isinstance(item, dict) and item.get("artifact_id")],
        },
        "map": {
            "root_node_id": str(map_payload.get("root_node_id") or ""),
            "node_count": node_count,
            "relationship_count": relationship_count,
            "segment_candidate_count": segment_candidate_count,
        },
        "next_actions": {
            "if_approved": "continue_pipeline",
            "if_rejected": "mark_review_failed_retryable",
        },
    }
    approval_request = ApprovalRequested(
        approval_id=f"approval-{uuid4().hex[:12]}",
        workflow_id=str(payload.get("workflow_id") or payload.get("artifact_id") or "workflow:parse-review"),
        step_id=str(payload.get("step_id") or "map.conceptual.lift.surface_fragments"),
        requested_by="executor://agent-env-primary",
        summary=summary,
        details=details,
    ).model_dump(mode="json")
    review = {
        "executor_id": "executor://agent-env-primary",
        "executor_kind": "agent-executor",
        "agent_spec": agent_spec,
        "decision": decision,
        "confidence": confidence,
        "findings": findings,
        "suggested_repairs": suggested_repairs,
        "elicitation_summary": summary,
        "elicitation_details": details,
    }
    elicitation: dict[str, Any] = {"approval_mode": approval_mode, "approval_request": approval_request}
    if approval_mode == "auto_approve":
        elicitation["approval_resolution"] = ApprovalResolved(
            approval_id=approval_request["approval_id"],
            workflow_id=approval_request["workflow_id"],
            step_id=approval_request["step_id"],
            resolved_by="system:auto-approve",
            approved=True,
            comment="Auto-approved by IKAM parse review policy",
        ).model_dump(mode="json")
    review["elicitation"] = elicitation
    return review
