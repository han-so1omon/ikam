from __future__ import annotations

import datetime as dt
import json
import uuid
from typing import Any, Dict, Literal, Optional, Union

from pydantic import BaseModel, Field, ConfigDict

from ikam.graph import Artifact, StoredFragment, _cas_hex

from .schema import ArtifactRef, PlanRef


PLAN_VERIFICATION_REPORT_SCHEMA_ID: str = "narraciones/ikam-plan-verification-report@1"


class PlanVerificationReport(BaseModel):
    """Deterministic verification report for plan milestone execution."""

    model_config = ConfigDict(extra="forbid")

    schema_id: Literal["narraciones/ikam-plan-verification-report@1"] = Field(
        default=PLAN_VERIFICATION_REPORT_SCHEMA_ID,
        alias="schema",
        description="Plan verification report schema id",
    )

    plan: PlanRef
    milestone: PlanRef

    status: Literal["completed", "blocked"]
    summary: str = Field(default="", description="Short summary of verification outcome")

    proposed_by: Optional[str] = Field(
        default=None, description="Agent/service identifier that produced this report"
    )
    command: Optional[str] = Field(default=None, description="Command executed")
    execution_id: Optional[str] = Field(default=None, description="Execution correlation id")
    error: Optional[str] = Field(default=None, description="Error summary if blocked")
    details: Dict[str, Any] = Field(default_factory=dict)
    created_at_ms: Optional[int] = Field(default=None)


def _canonicalize_json(payload: Dict[str, Any]) -> bytes:
    stable_json = json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return stable_json.encode("utf-8")


def canonicalize_plan_verification_report_json(
    report: Union[PlanVerificationReport, Dict[str, Any]]
) -> bytes:
    report_model = (
        report if isinstance(report, PlanVerificationReport) else PlanVerificationReport.model_validate(report)
    )
    payload = report_model.model_dump(mode="json", by_alias=True, exclude_none=True)
    return _canonicalize_json(payload)


def plan_verification_report_cas_id(report_bytes: bytes) -> str:
    return _cas_hex(report_bytes)


def plan_verification_report_artifact_uuid(report_bytes: bytes) -> uuid.UUID:
    fragment_id = plan_verification_report_cas_id(report_bytes)
    return uuid.uuid5(uuid.NAMESPACE_URL, f"narraciones/ikam-plan-verification-report:{fragment_id}")


def plan_verification_report_to_ikam_artifact(
    report: Union[PlanVerificationReport, Dict[str, Any]],
    *,
    created_at: Optional[dt.datetime] = None,
) -> tuple[Artifact, StoredFragment, ArtifactRef]:
    report_model = (
        report if isinstance(report, PlanVerificationReport) else PlanVerificationReport.model_validate(report)
    )
    report_bytes = canonicalize_plan_verification_report_json(report_model)
    fragment = StoredFragment.from_bytes(report_bytes, mime_type="application/json")
    artifact_id = plan_verification_report_artifact_uuid(report_bytes)

    artifact_created_at = created_at or dt.datetime.now(dt.timezone.utc)
    artifact = Artifact(
        id=str(artifact_id),
        kind="file",
        title="Plan Verification Report",
        root_fragment_id=fragment.id,
        created_at=artifact_created_at,
    )
    return artifact, fragment, ArtifactRef(artifact_id=str(artifact_id), fragment_id=fragment.id)
