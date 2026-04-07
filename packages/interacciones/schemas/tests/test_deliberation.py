"""Unit tests for deliberation envelope schemas."""

import pytest
from pydantic import ValidationError

from interacciones.schemas import (
    DeliberationEnvelope,
    DeliberationEvidence,
    DeliberationEvidenceKind,
    DeliberationPhase,
    DeliberationStatus,
)


def test_deliberation_envelope_round_trip() -> None:
    env = DeliberationEnvelope(
        runId="123",
        projectId="proj-1",
        ts=1700000000000,
        phase=DeliberationPhase.PLAN,
        status=DeliberationStatus.STARTED,
        summary="Planning started",
        evidence=[
            DeliberationEvidence(kind=DeliberationEvidenceKind.ARTIFACT, ref="plan:abc", label="plan")
        ],
    )

    dumped = env.model_dump(mode="json")
    assert dumped == {
        "runId": "123",
        "projectId": "proj-1",
        "ts": 1700000000000,
        "phase": "plan",
        "status": "started",
        "summary": "Planning started",
        "details": None,
        "evidence": [{"kind": "artifact", "ref": "plan:abc", "label": "plan"}],
    }

    restored = DeliberationEnvelope.model_validate(dumped)
    assert restored == env


def test_deliberation_envelope_rejects_unknown_fields() -> None:
    with pytest.raises(ValidationError):
        DeliberationEnvelope.model_validate(
            {
                "runId": "123",
                "projectId": "proj-1",
                "ts": 1700000000000,
                "phase": "plan",
                "status": "started",
                "summary": "Planning started",
                "extra_field": "nope",
            }
        )


def test_deliberation_envelope_rejects_invalid_enums() -> None:
    with pytest.raises(ValidationError):
        DeliberationEnvelope.model_validate(
            {
                "runId": "123",
                "projectId": "proj-1",
                "ts": 1700000000000,
                "phase": "invalid",
                "status": "started",
                "summary": "Planning started",
            }
        )

    with pytest.raises(ValidationError):
        DeliberationEnvelope.model_validate(
            {
                "runId": "123",
                "projectId": "proj-1",
                "ts": 1700000000000,
                "phase": "plan",
                "status": "nope",
                "summary": "Planning started",
            }
        )


def test_deliberation_evidence_rejects_unknown_fields() -> None:
    with pytest.raises(ValidationError):
        DeliberationEvidence.model_validate(
            {"kind": "artifact", "ref": "x", "label": None, "extra": "nope"}
        )
