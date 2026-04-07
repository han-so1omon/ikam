"""Deliberation breadcrumb envelope models.

These models mirror the canonical Avro schema in `schemas/deliberation-envelope.avsc`.
They are intentionally *not* chain-of-thought: they carry a safe summary and
optional evidence references.
"""

from __future__ import annotations

from enum import Enum
from typing import Optional, List, Iterable, Dict, Any

from pydantic import BaseModel, ConfigDict


class DeliberationPhase(str, Enum):
    PLAN = "plan"
    HYPOTHESIS = "hypothesis"
    EXPERIMENT = "experiment"
    VALIDATE = "validate"
    DECIDE = "decide"
    SUMMARIZE = "summarize"


class DeliberationStatus(str, Enum):
    STARTED = "started"
    UPDATE = "update"
    COMPLETE = "complete"
    FAILED = "failed"


class DeliberationEvidenceKind(str, Enum):
    IKAM_FRAGMENT = "ikam_fragment"
    ARTIFACT = "artifact"
    PROJECTION = "projection"
    URL = "url"


class DeliberationEvidence(BaseModel):
    kind: DeliberationEvidenceKind
    ref: str
    label: Optional[str] = None

    model_config = ConfigDict(extra="forbid")


class DeliberationEnvelope(BaseModel):
    runId: str
    projectId: str
    ts: int
    phase: DeliberationPhase
    status: DeliberationStatus
    summary: str
    details: Optional[str] = None
    evidence: Optional[List[DeliberationEvidence]] = None

    model_config = ConfigDict(extra="forbid")


def build_deliberation_envelope(
    *,
    run_id: str,
    project_id: str,
    ts: int,
    phase: DeliberationPhase | str,
    status: DeliberationStatus | str,
    summary: str,
    details: Optional[str] = None,
    evidence: Optional[Iterable[DeliberationEvidence | Dict[str, Any]]] = None,
) -> DeliberationEnvelope:
    """Build a validated deliberation envelope for system_event payloads."""
    phase_enum = phase if isinstance(phase, DeliberationPhase) else DeliberationPhase(phase)
    status_enum = status if isinstance(status, DeliberationStatus) else DeliberationStatus(status)
    evidence_models: Optional[List[DeliberationEvidence]] = None
    if evidence is not None:
        evidence_models = [
            item if isinstance(item, DeliberationEvidence) else DeliberationEvidence.model_validate(item)
            for item in evidence
        ]
    return DeliberationEnvelope(
        runId=str(run_id),
        projectId=str(project_id),
        ts=int(ts),
        phase=phase_enum,
        status=status_enum,
        summary=summary,
        details=details,
        evidence=evidence_models,
    )


def build_deliberation_system_event(
    *,
    project_id: str,
    session_id: Optional[str],
    parent_id: Optional[str],
    run_id: str,
    ts: int,
    phase: DeliberationPhase | str,
    status: DeliberationStatus | str,
    summary: str,
    details: Optional[str] = None,
    evidence: Optional[Iterable[DeliberationEvidence | Dict[str, Any]]] = None,
) -> Dict[str, Any]:
    """Build a deliberation system_event envelope matching the SSE/Kafka contract."""
    envelope = build_deliberation_envelope(
        run_id=run_id,
        project_id=project_id,
        ts=ts,
        phase=phase,
        status=status,
        summary=summary,
        details=details,
        evidence=evidence,
    )
    return {
        "interaction_id": None,
        "project_id": project_id,
        "session_id": session_id,
        "scope": "system",
        "type": "system_event",
        "content": summary,
        "metadata": {
            "event_type": "deliberation",
            "deliberation": envelope.model_dump(mode="json"),
        },
        "parent_id": parent_id,
        "created_at": int(ts),
    }
