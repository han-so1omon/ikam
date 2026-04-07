from __future__ import annotations

from dataclasses import asdict
from typing import Any

from interacciones.schemas.decision_trace import (
    DecisionTrace,
    DecisionTraceCandidate,
    DecisionTraceQuery,
)
from interacciones.streaming.progress_event import ProgressEvent, ProgressEventKind


def build_decision_trace_envelope(
    *,
    matched_at_ms: int,
    mode: str,
    selected_agent_id: str,
    candidates: list[dict[str, Any]],
    tags: list[str] | None = None,
) -> dict[str, Any]:
    trace = DecisionTrace(
        version="1.0",
        kind="relation_enrichment",
        matched_at_ms=matched_at_ms,
        query=DecisionTraceQuery(domain="ikam-enrichment", action=mode, tags=list(tags or []), require_healthy=True),
        candidates=[
            DecisionTraceCandidate(
                agent_id=str(item.get("agent_id") or "relation-orchestrator"),
                score=float(item.get("score", 1.0)),
                reasons=[str(reason) for reason in (item.get("reasons") or [])],
                status=item.get("status"),
                in_flight=item.get("in_flight"),
            )
            for item in candidates
        ],
        selected_agent_id=selected_agent_id,
    )
    return trace.model_dump(mode="json")


def build_progress_event_envelope(
    *,
    event_id: str,
    operation_id: str,
    project_id: str,
    ts: int,
    kind: str,
    message: str,
    progress: float,
    stage: str,
    payload: dict[str, Any] | None = None,
) -> dict[str, Any]:
    event = ProgressEvent(
        event_id=event_id,
        operation_id=operation_id,
        project_id=project_id,
        ts=ts,
        kind=ProgressEventKind(kind),
        message=message,
        progress=progress,
        stage=stage,
        payload=payload,
        actor="relation-orchestrator",
    )
    return asdict(event)
