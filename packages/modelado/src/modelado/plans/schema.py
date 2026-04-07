from __future__ import annotations

import json
import uuid
from typing import Any, Dict, List, Literal, Optional, Union, Mapping

from pydantic import BaseModel, Field, ConfigDict

from ikam.graph import _cas_hex


PLAN_SCHEMA_ID: str = "narraciones/ikam-plan@1"
PLAN_ENVELOPE_SCHEMA_ID: str = "narraciones/ikam-plan-envelope@1"
PLAN_AMENDMENT_SCHEMA_ID: str = "narraciones/ikam-plan-amendment@1"
PLAN_MILESTONE_ENVELOPE_SCHEMA_ID: str = "narraciones/ikam-plan-milestone-envelope@1"

# Petri net (Net + Run) schema ids
PETRI_NET_ENVELOPE_SCHEMA_ID: str = "modelado/petri-net-envelope@1"
PETRI_NET_PLACE_SCHEMA_ID: str = "modelado/petri-net-place@1"
PETRI_NET_TRANSITION_SCHEMA_ID: str = "modelado/petri-net-transition@1"
PETRI_NET_ARC_SCHEMA_ID: str = "modelado/petri-net-arc@1"
PETRI_NET_MARKING_SCHEMA_ID: str = "modelado/petri-net-marking@1"
PETRI_NET_SECTION_SCHEMA_ID: str = "modelado/petri-net-section@1"
PETRI_RUN_ENVELOPE_SCHEMA_ID: str = "modelado/petri-net-run-envelope@1"
PETRI_RUN_FIRING_SCHEMA_ID: str = "modelado/petri-net-run-firing@1"


class PlanRef(BaseModel):
    model_config = ConfigDict(extra="forbid")

    artifact_id: str = Field(..., description="Plan artifact UUID")
    fragment_id: str = Field(..., description="CAS fragment id for the referenced section")


def normalize_plan_ref(value: Any) -> PlanRef:
    """Normalize a PlanRef-like input to the canonical PlanRef model."""

    if isinstance(value, PlanRef):
        return value
    if not isinstance(value, dict):
        raise ValueError("plan_ref must be an object")

    plan_artifact_id = value.get("plan_artifact_id")
    fragment_id = value.get("fragment_id")

    plan_artifact_id = str(plan_artifact_id or "").strip()
    fragment_id = str(fragment_id or "").strip()

    if not plan_artifact_id or not fragment_id:
        raise ValueError("plan_ref must include plan_artifact_id and fragment_id")
    if plan_artifact_id.startswith("plan:"):
        raise ValueError("Legacy plan:{sha} identifiers are not allowed; expected UUID plan_artifact_id")

    return PlanRef(artifact_id=plan_artifact_id, fragment_id=fragment_id)


def plan_ref_from_metadata(
    metadata: Mapping[str, Any] | None,
    *,
    required: bool = False,
) -> Optional[PlanRef]:
    """Extract and normalize PlanRef from metadata when present."""

    if not isinstance(metadata, Mapping):
        if required:
            raise ValueError("Missing required metadata.plan_ref")
        return None

    plan_ref = metadata.get("plan_ref")
    if plan_ref is None:
        if required:
            raise ValueError("Missing required metadata.plan_ref")
        return None

    return normalize_plan_ref(plan_ref)


class ArtifactRef(BaseModel):
    model_config = ConfigDict(extra="forbid")

    artifact_id: str = Field(..., description="IKAM artifact UUID")
    fragment_id: str = Field(..., description="CAS fragment id")


class PlanPatchOp(BaseModel):
    """Minimal RFC6902-style JSON Patch operation."""

    model_config = ConfigDict(extra="forbid")

    op: Literal["add", "replace", "remove"]
    path: str
    value: Optional[Any] = None


class PlanAmendment(BaseModel):
    """Deterministic plan amendment."""

    model_config = ConfigDict(extra="forbid")

    schema_id: Literal["narraciones/ikam-plan-amendment@1"] = Field(
        default=PLAN_AMENDMENT_SCHEMA_ID,
        alias="schema",
        description="Plan amendment schema id",
    )

    amendment_id: Optional[str] = Field(
        default=None,
        description="Deterministic amendment id (caller-supplied). If omitted, use CAS id.",
    )

    proposed_by: Optional[str] = Field(
        default=None,
        description="Agent/service identifier that proposed the amendment",
    )

    targets: List[PlanRef] = Field(
        default_factory=list,
        description="Optional list of PlanRefs this amendment intends to affect",
    )

    rationale_summary: str = Field(
        default="",
        description="1–3 sentence safe summary (no chain-of-thought)",
    )

    verification: List[ArtifactRef] = Field(
        default_factory=list,
        description="References to verification/evidence artifacts",
    )

    risk_flags: List[str] = Field(
        default_factory=list,
        description="Optional risk markers (e.g. requires_user_confirmation)",
    )

    base_plan: PlanRef = Field(..., description="Reference to the base plan artifact/root fragment")
    delta: List[PlanPatchOp] = Field(default_factory=list, description="Patch operations")


class PlanEnvelope(BaseModel):
    """Internal fragment that encodes the plan header + milestone ordering."""

    model_config = ConfigDict(extra="forbid")

    schema_id: Literal["narraciones/ikam-plan-envelope@1"] = Field(
        default=PLAN_ENVELOPE_SCHEMA_ID,
        alias="schema",
        description="Plan envelope schema id",
    )

    scope_id: str
    title: str
    goal: str

    milestone_ids: List[str] = Field(
        default_factory=list,
        description="Milestone IDs in the plan's canonical order",
    )


class Validation(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: str = Field(..., description="Validation identifier")
    description: str = Field(..., description="What this validation checks")


class FailurePath(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: str = Field(..., description="Failure path identifier")
    when: str = Field(..., description="Condition that triggers this failure path")
    action: str = Field(
        ..., description="Actionable next step (clarification, amendment, retry, etc.)"
    )


class Milestone(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: str = Field(..., description="Milestone identifier")
    title: str = Field(..., description="Milestone title")
    description: str = Field(default="", description="Milestone description")

    depends_on: List[str] = Field(
        default_factory=list, description="Milestone IDs that must complete first"
    )

    success_criteria: List[str] = Field(
        default_factory=list, description="Observable success criteria"
    )

    validations: List[Validation] = Field(default_factory=list)
    failure_paths: List[FailurePath] = Field(default_factory=list)


class PlanMilestoneEnvelope(BaseModel):
    """Internal fragment that encodes milestone core fields + refs to sub-fragments."""

    model_config = ConfigDict(extra="forbid")

    schema_id: Literal["narraciones/ikam-plan-milestone-envelope@1"] = Field(
        default=PLAN_MILESTONE_ENVELOPE_SCHEMA_ID,
        alias="schema",
        description="Milestone envelope schema id",
    )

    id: str
    title: str
    description: str = ""

    depends_on: List[str] = Field(default_factory=list)
    success_criteria: List[str] = Field(default_factory=list)

    validation_fragment_ids: List[str] = Field(default_factory=list)
    failure_path_fragment_ids: List[str] = Field(default_factory=list)


class Plan(BaseModel):
    """Canonical, execution-oriented plan schema."""

    model_config = ConfigDict(extra="forbid")

    schema_id: Literal["narraciones/ikam-plan@1"] = Field(
        default=PLAN_SCHEMA_ID,
        alias="schema",
        description="Plan schema id",
    )

    scope_id: str = Field(
        ..., description="Semantic scope id (e.g., conversation-bounded scope)"
    )
    title: str = Field(..., description="Human readable title")
    goal: str = Field(..., description="What the plan is trying to accomplish")

    milestones: List[Milestone] = Field(default_factory=list)


class PetriNetEnvelope(BaseModel):
    """Root fragment for a Petri net definition."""

    model_config = ConfigDict(extra="forbid")

    schema_id: Literal["modelado/petri-net-envelope@1"] = Field(
        default=PETRI_NET_ENVELOPE_SCHEMA_ID,
        alias="schema",
        description="Petri plan envelope schema id",
    )

    project_id: str
    scope_id: str
    title: str
    goal: str

    place_fragment_ids: List[str] = Field(default_factory=list)
    transition_fragment_ids: List[str] = Field(default_factory=list)
    arc_fragment_ids: List[str] = Field(default_factory=list)
    initial_marking_fragment_id: str
    index_fragment_id: Optional[str] = None
    version: str = "1"


class PetriNetPlace(BaseModel):
    model_config = ConfigDict(extra="forbid")

    schema_id: Literal["modelado/petri-net-place@1"] = Field(
        default=PETRI_NET_PLACE_SCHEMA_ID,
        alias="schema",
        description="Petri plan place schema id",
    )

    place_id: str
    label: str
    capacity: Optional[int] = None
    tags: List[str] = Field(default_factory=list)
    metadata: Dict[str, Any] = Field(default_factory=dict)


class PetriNetArcEndpoint(BaseModel):
    model_config = ConfigDict(extra="forbid")

    place_id: str
    weight: int = 1


class PetriNetTransition(BaseModel):
    model_config = ConfigDict(extra="forbid")

    schema_id: Literal["modelado/petri-net-transition@1"] = Field(
        default=PETRI_NET_TRANSITION_SCHEMA_ID,
        alias="schema",
        description="Petri plan transition schema id",
    )

    transition_id: str
    label: str
    operation_ref: str
    guard: Optional[Dict[str, Any]] = None
    inputs: List[PetriNetArcEndpoint] = Field(default_factory=list)
    outputs: List[PetriNetArcEndpoint] = Field(default_factory=list)
    tags: List[str] = Field(default_factory=list)
    metadata: Dict[str, Any] = Field(default_factory=dict)


class PetriNetArc(BaseModel):
    model_config = ConfigDict(extra="forbid")

    schema_id: Literal["modelado/petri-net-arc@1"] = Field(
        default=PETRI_NET_ARC_SCHEMA_ID,
        alias="schema",
        description="Petri plan arc schema id",
    )

    from_kind: Literal["place", "transition"]
    from_id: str
    to_kind: Literal["place", "transition"]
    to_id: str
    weight: int = 1


class PetriNetMarking(BaseModel):
    model_config = ConfigDict(extra="forbid")

    schema_id: Literal["modelado/petri-net-marking@1"] = Field(
        default=PETRI_NET_MARKING_SCHEMA_ID,
        alias="schema",
        description="Petri plan marking schema id",
    )

    tokens: Dict[str, int] = Field(default_factory=dict)
    meta: Dict[str, Any] = Field(default_factory=dict)


class PetriNetSection(BaseModel):
    """Derived fragment representing a section grouping of transitions."""

    model_config = ConfigDict(extra="forbid")

    schema_id: Literal["modelado/petri-net-section@1"] = Field(
        default=PETRI_NET_SECTION_SCHEMA_ID,
        alias="schema",
        description="Petri plan section schema id",
    )

    transition_ids: List[str] = Field(default_factory=list)
    title: Optional[str] = None


class PlanTransitionRef(BaseModel):
    model_config = ConfigDict(extra="forbid")

    plan_artifact_id: str
    transition_fragment_id: str
    transition_id: str


class PetriNetRunEnvelope(BaseModel):
    model_config = ConfigDict(extra="forbid")

    schema_id: Literal["modelado/petri-net-run-envelope@1"] = Field(
        default=PETRI_RUN_ENVELOPE_SCHEMA_ID,
        alias="schema",
        description="PetriNetRun envelope schema id",
    )

    project_id: str
    scope_id: str
    plan_ref: PlanRef
    run_id: str
    policy: Dict[str, Any] = Field(default_factory=dict)
    started_at: Optional[str] = None
    ended_at: Optional[str] = None
    initial_marking_fragment_id: str
    final_marking_fragment_id: Optional[str] = None
    firing_fragment_ids: List[str] = Field(default_factory=list)


class PetriNetRunFiring(BaseModel):
    model_config = ConfigDict(extra="forbid")

    schema_id: Literal["modelado/petri-net-run-firing@1"] = Field(
        default=PETRI_RUN_FIRING_SCHEMA_ID,
        alias="schema",
        description="PetriNetRun firing schema id",
    )

    firing_id: str
    transition_ref: PlanTransitionRef
    marking_before_fragment_id: str
    marking_after_fragment_id: str
    status: Literal["success", "failed", "skipped"]
    error: Optional[str] = None
    effects: Dict[str, Any] = Field(default_factory=dict)
    ts_ms: int


def canonicalize_json(payload: Dict[str, Any]) -> bytes:
    stable_json = json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return stable_json.encode("utf-8")


def canonicalize_plan_json(plan: Union[Plan, Dict[str, Any]]) -> bytes:
    """Serialize a plan to deterministic UTF-8 JSON bytes."""

    plan_model = plan if isinstance(plan, Plan) else Plan.model_validate(plan)
    payload = plan_model.model_dump(mode="json", by_alias=True, exclude_none=True)
    return canonicalize_json(payload)


def canonicalize_petri_net_envelope_json(envelope: Union[PetriNetEnvelope, Dict[str, Any]]) -> bytes:
    envelope_model = (
        envelope if isinstance(envelope, PetriNetEnvelope) else PetriNetEnvelope.model_validate(envelope)
    )
    payload = envelope_model.model_dump(mode="json", by_alias=True, exclude_none=True)
    return canonicalize_json(payload)


def canonicalize_petri_net_place_json(place: Union[PetriNetPlace, Dict[str, Any]]) -> bytes:
    place_model = place if isinstance(place, PetriNetPlace) else PetriNetPlace.model_validate(place)
    payload = place_model.model_dump(mode="json", by_alias=True, exclude_none=True)
    return canonicalize_json(payload)


def canonicalize_petri_net_transition_json(
    transition: Union[PetriNetTransition, Dict[str, Any]]
) -> bytes:
    transition_model = (
        transition
        if isinstance(transition, PetriNetTransition)
        else PetriNetTransition.model_validate(transition)
    )
    payload = transition_model.model_dump(mode="json", by_alias=True, exclude_none=True)
    return canonicalize_json(payload)


def canonicalize_petri_net_arc_json(arc: Union[PetriNetArc, Dict[str, Any]]) -> bytes:
    arc_model = arc if isinstance(arc, PetriNetArc) else PetriNetArc.model_validate(arc)
    payload = arc_model.model_dump(mode="json", by_alias=True, exclude_none=True)
    return canonicalize_json(payload)


def canonicalize_petri_net_marking_json(marking: Union[PetriNetMarking, Dict[str, Any]]) -> bytes:
    marking_model = (
        marking if isinstance(marking, PetriNetMarking) else PetriNetMarking.model_validate(marking)
    )
    payload = marking_model.model_dump(mode="json", by_alias=True, exclude_none=True)
    return canonicalize_json(payload)


def canonicalize_petri_net_section_json(section: Union[PetriNetSection, Dict[str, Any]]) -> bytes:
    section_model = (
        section if isinstance(section, PetriNetSection) else PetriNetSection.model_validate(section)
    )
    payload = section_model.model_dump(mode="json", by_alias=True, exclude_none=True)
    return canonicalize_json(payload)


def canonicalize_petri_run_envelope_json(envelope: Union[PetriNetRunEnvelope, Dict[str, Any]]) -> bytes:
    envelope_model = (
        envelope if isinstance(envelope, PetriNetRunEnvelope) else PetriNetRunEnvelope.model_validate(envelope)
    )
    payload = envelope_model.model_dump(mode="json", by_alias=True, exclude_none=True)
    return canonicalize_json(payload)


def canonicalize_petri_run_firing_json(firing: Union[PetriNetRunFiring, Dict[str, Any]]) -> bytes:
    firing_model = (
        firing if isinstance(firing, PetriNetRunFiring) else PetriNetRunFiring.model_validate(firing)
    )
    payload = firing_model.model_dump(mode="json", by_alias=True, exclude_none=True)
    return canonicalize_json(payload)


def canonicalize_plan_amendment_json(amendment: Union[PlanAmendment, Dict[str, Any]]) -> bytes:
    amendment_model = (
        amendment if isinstance(amendment, PlanAmendment) else PlanAmendment.model_validate(amendment)
    )
    payload = amendment_model.model_dump(mode="json", by_alias=True, exclude_none=True)
    return canonicalize_json(payload)


def amendment_cas_id(amendment_bytes: bytes) -> str:
    return _cas_hex(amendment_bytes)


def amendment_artifact_uuid(amendment_bytes: bytes) -> uuid.UUID:
    fragment_id = amendment_cas_id(amendment_bytes)
    return uuid.uuid5(uuid.NAMESPACE_URL, f"narraciones/ikam-plan-amendment:{fragment_id}")


def canonicalize_plan_envelope_json(envelope: Union[PlanEnvelope, Dict[str, Any]]) -> bytes:
    envelope_model = envelope if isinstance(envelope, PlanEnvelope) else PlanEnvelope.model_validate(envelope)
    payload = envelope_model.model_dump(mode="json", by_alias=True, exclude_none=True)
    return canonicalize_json(payload)


def canonicalize_milestone_json(milestone: Union[Milestone, Dict[str, Any]]) -> bytes:
    milestone_model = milestone if isinstance(milestone, Milestone) else Milestone.model_validate(milestone)
    payload = milestone_model.model_dump(mode="json", by_alias=True, exclude_none=True)
    return canonicalize_json(payload)


def canonicalize_validation_json(validation: Union[Validation, Dict[str, Any]]) -> bytes:
    validation_model = (
        validation if isinstance(validation, Validation) else Validation.model_validate(validation)
    )
    payload = validation_model.model_dump(mode="json", by_alias=True, exclude_none=True)
    return canonicalize_json(payload)


def canonicalize_failure_path_json(failure_path: Union[FailurePath, Dict[str, Any]]) -> bytes:
    failure_model = (
        failure_path
        if isinstance(failure_path, FailurePath)
        else FailurePath.model_validate(failure_path)
    )
    payload = failure_model.model_dump(mode="json", by_alias=True, exclude_none=True)
    return canonicalize_json(payload)


def canonicalize_milestone_envelope_json(envelope: Union[PlanMilestoneEnvelope, Dict[str, Any]]) -> bytes:
    envelope_model = (
        envelope
        if isinstance(envelope, PlanMilestoneEnvelope)
        else PlanMilestoneEnvelope.model_validate(envelope)
    )
    payload = envelope_model.model_dump(mode="json", by_alias=True, exclude_none=True)
    return canonicalize_json(payload)


def require_unique_place_ids(places: List[PetriNetPlace]) -> None:
    seen: set[str] = set()
    for place in places:
        if place.place_id in seen:
            raise ValueError(f"Duplicate place_id in petri net: {place.place_id}")
        seen.add(place.place_id)


def require_unique_transition_ids(transitions: List[PetriNetTransition]) -> None:
    seen: set[str] = set()
    for transition in transitions:
        if transition.transition_id in seen:
            raise ValueError(
                f"Duplicate transition_id in petri net: {transition.transition_id}"
            )
        seen.add(transition.transition_id)


def require_transition_place_refs(
    transitions: List[PetriNetTransition],
    place_ids: List[str],
) -> None:
    known = set(place_ids)
    for transition in transitions:
        for endpoint in [*transition.inputs, *transition.outputs]:
            if endpoint.place_id not in known:
                raise ValueError(
                    f"Unknown place_id in transition '{transition.transition_id}': {endpoint.place_id}"
                )


def require_non_negative_marking(marking: PetriNetMarking) -> None:
    for place_id, count in marking.tokens.items():
        if count < 0:
            raise ValueError(f"Negative marking for place '{place_id}': {count}")


def require_unique_milestone_ids(plan: Plan) -> None:
    seen: set[str] = set()
    for ms in plan.milestones:
        if ms.id in seen:
            raise ValueError(f"Duplicate milestone id in plan: {ms.id}")
        seen.add(ms.id)


def plan_cas_id(plan_bytes: bytes) -> str:
    """Return the CAS fragment id for the canonical plan bytes."""

    return _cas_hex(plan_bytes)


def plan_artifact_uuid(plan_bytes: bytes) -> uuid.UUID:
    """Derive a stable UUID for the plan artifact."""

    fragment_id = plan_cas_id(plan_bytes)
    return uuid.uuid5(uuid.NAMESPACE_URL, f"narraciones/ikam-plan:{fragment_id}")


def petri_net_cas_id(envelope_bytes: bytes) -> str:
    """Return the CAS fragment id for the petri net envelope bytes."""

    return _cas_hex(envelope_bytes)


def petri_net_artifact_uuid(envelope_bytes: bytes) -> uuid.UUID:
    """Derive a stable UUID for the Petri net artifact."""

    fragment_id = petri_net_cas_id(envelope_bytes)
    return uuid.uuid5(uuid.NAMESPACE_URL, f"modelado/petri-net:{fragment_id}")


def petri_run_artifact_uuid(envelope_bytes: bytes) -> uuid.UUID:
    """Derive a stable UUID for the PetriNetRun artifact."""

    fragment_id = petri_net_cas_id(envelope_bytes)
    return uuid.uuid5(uuid.NAMESPACE_URL, f"modelado/petri-net-run:{fragment_id}")
