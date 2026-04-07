from __future__ import annotations

import datetime as dt
import json
from typing import Any, Dict, List, Optional, Tuple, Union

from ikam.graph import Artifact, StoredFragment

from .schema import (
    ArtifactRef,
    FailurePath,
    Milestone,
    Plan,
    PlanEnvelope,
    PlanAmendment,
    PlanMilestoneEnvelope,
    PlanRef,
    PetriNetEnvelope,
    PetriNetMarking,
    PetriNetPlace,
    PetriNetTransition,
    PetriNetArc,
    PetriNetSection,
    PetriNetRunEnvelope,
    PetriNetRunFiring,
    Validation,
    amendment_artifact_uuid,
    canonicalize_plan_amendment_json,
    canonicalize_failure_path_json,
    canonicalize_milestone_envelope_json,
    canonicalize_milestone_json,
    canonicalize_plan_envelope_json,
    canonicalize_plan_json,
    canonicalize_petri_net_envelope_json,
    canonicalize_petri_net_place_json,
    canonicalize_petri_net_transition_json,
    canonicalize_petri_net_arc_json,
    canonicalize_petri_net_marking_json,
    canonicalize_petri_net_section_json,
    canonicalize_petri_run_envelope_json,
    canonicalize_petri_run_firing_json,
    canonicalize_validation_json,
    plan_artifact_uuid,
    petri_net_artifact_uuid,
    petri_run_artifact_uuid,
    require_unique_milestone_ids,
    require_non_negative_marking,
    require_transition_place_refs,
    require_unique_place_ids,
    require_unique_transition_ids,
)


def plan_to_ikam_artifact(
    plan: Union[Plan, Dict[str, Any]],
    *,
    created_at: Optional[dt.datetime] = None,

) -> Tuple[Artifact, List[StoredFragment], PlanRef, Dict[str, PlanRef]]:
    """Build an IKAM storage artifact + fragments for a canonical plan."""

    plan_model = plan if isinstance(plan, Plan) else Plan.model_validate(plan)
    require_unique_milestone_ids(plan_model)

    plan_bytes = canonicalize_plan_json(plan_model)
    artifact_id = plan_artifact_uuid(plan_bytes)

    envelope = PlanEnvelope(
        scope_id=plan_model.scope_id,
        title=plan_model.title,
        goal=plan_model.goal,
        milestone_ids=[ms.id for ms in plan_model.milestones],
    )
    envelope_bytes = canonicalize_plan_envelope_json(envelope)
    envelope_fragment = StoredFragment.from_bytes(envelope_bytes, mime_type="application/json")

    milestone_fragments: List[StoredFragment] = []
    validation_fragments: List[StoredFragment] = []
    failure_path_fragments: List[StoredFragment] = []
    milestone_refs: Dict[str, PlanRef] = {}
    for milestone in plan_model.milestones:
        milestone_model = milestone if isinstance(milestone, Milestone) else Milestone.model_validate(milestone)

        milestone_validation_fragment_ids: List[str] = []
        for v in milestone_model.validations:
            v_bytes = canonicalize_validation_json(v)
            v_fragment = StoredFragment.from_bytes(v_bytes, mime_type="application/json")
            validation_fragments.append(v_fragment)
            milestone_validation_fragment_ids.append(v_fragment.id)

        milestone_failure_fragment_ids: List[str] = []
        for fp in milestone_model.failure_paths:
            fp_bytes = canonicalize_failure_path_json(fp)
            fp_fragment = StoredFragment.from_bytes(fp_bytes, mime_type="application/json")
            failure_path_fragments.append(fp_fragment)
            milestone_failure_fragment_ids.append(fp_fragment.id)

        ms_envelope = PlanMilestoneEnvelope(
            id=milestone_model.id,
            title=milestone_model.title,
            description=milestone_model.description,
            depends_on=milestone_model.depends_on,
            success_criteria=milestone_model.success_criteria,
            validation_fragment_ids=milestone_validation_fragment_ids,
            failure_path_fragment_ids=milestone_failure_fragment_ids,
        )
        ms_envelope_bytes = canonicalize_milestone_envelope_json(ms_envelope)
        ms_fragment = StoredFragment.from_bytes(ms_envelope_bytes, mime_type="application/json")
        milestone_fragments.append(ms_fragment)

        milestone_refs[milestone_model.id] = PlanRef(artifact_id=str(artifact_id), fragment_id=ms_fragment.id)

    fragments: List[StoredFragment] = [
        envelope_fragment,
        *milestone_fragments,
        *validation_fragments,
        *failure_path_fragments,
    ]
    artifact_created_at = created_at or dt.datetime.now(dt.timezone.utc)
    artifact = Artifact(
        id=str(artifact_id),
        kind="file",
        title=plan_model.title,
        root_fragment_id=envelope_fragment.id,
        created_at=artifact_created_at,
    )

    plan_root_ref = PlanRef(artifact_id=str(artifact_id), fragment_id=envelope_fragment.id)

    return artifact, fragments, plan_root_ref, milestone_refs


def reconstruct_plan_from_fragments(
    *,
    envelope_fragment: StoredFragment,
    milestone_fragments: List[StoredFragment],
) -> Plan:
    envelope = PlanEnvelope.model_validate_json(envelope_fragment.bytes)

    fragments_by_id: Dict[str, StoredFragment] = {f.id: f for f in milestone_fragments}

    milestone_envelopes_by_id: Dict[str, PlanMilestoneEnvelope] = {}
    legacy_milestones_by_id: Dict[str, Milestone] = {}
    for frag in milestone_fragments:
        try:
            ms_env = PlanMilestoneEnvelope.model_validate_json(frag.bytes)
            milestone_envelopes_by_id[ms_env.id] = ms_env
            continue
        except Exception:
            pass

        try:
            ms = Milestone.model_validate_json(frag.bytes)
            legacy_milestones_by_id[ms.id] = ms
        except Exception:
            # Not a milestone envelope or a legacy milestone; ignore here.
            pass

    milestones: List[Milestone] = []
    if milestone_envelopes_by_id:
        for ms_id in envelope.milestone_ids:
            ms_env = milestone_envelopes_by_id[ms_id]
            validations: List[Validation] = []
            for frag_id in ms_env.validation_fragment_ids:
                validations.append(Validation.model_validate_json(fragments_by_id[frag_id].bytes))
            failure_paths: List[FailurePath] = []
            for frag_id in ms_env.failure_path_fragment_ids:
                failure_paths.append(FailurePath.model_validate_json(fragments_by_id[frag_id].bytes))

            milestones.append(
                Milestone(
                    id=ms_env.id,
                    title=ms_env.title,
                    description=ms_env.description,
                    depends_on=ms_env.depends_on,
                    success_criteria=ms_env.success_criteria,
                    validations=validations,
                    failure_paths=failure_paths,
                )
            )
    else:
        for ms_id in envelope.milestone_ids:
            milestones.append(legacy_milestones_by_id[ms_id])

    return Plan(
        scope_id=envelope.scope_id,
        title=envelope.title,
        goal=envelope.goal,
        milestones=milestones,
    )


def plan_amendment_to_ikam_artifact(
    amendment: Union[PlanAmendment, Dict[str, Any]],
    *,
    created_at: Optional[dt.datetime] = None,
) -> Tuple[Artifact, StoredFragment, ArtifactRef]:
    amendment_model = (
        amendment if isinstance(amendment, PlanAmendment) else PlanAmendment.model_validate(amendment)
    )
    amendment_bytes = canonicalize_plan_amendment_json(amendment_model)
    fragment = StoredFragment.from_bytes(amendment_bytes, mime_type="application/json")
    artifact_id = amendment_artifact_uuid(amendment_bytes)

    artifact_created_at = created_at or dt.datetime.now(dt.timezone.utc)
    artifact = Artifact(
        id=str(artifact_id),
        kind="file",
        title="Plan Amendment",
        root_fragment_id=fragment.id,
        created_at=artifact_created_at,
    )
    return artifact, fragment, ArtifactRef(artifact_id=str(artifact_id), fragment_id=fragment.id)


def petri_net_to_ikam_artifact(
    *,
    project_id: str,
    scope_id: str,
    title: str,
    goal: str,
    places: List[PetriNetPlace],
    transitions: List[PetriNetTransition],
    arcs: Optional[List[PetriNetArc]],
    marking: PetriNetMarking,
    sections: Optional[List[PetriNetSection]] = None,
    index_fragment: Optional[StoredFragment] = None,
    created_at: Optional[dt.datetime] = None,
) -> Tuple[Artifact, List[StoredFragment], PlanRef, Dict[str, PlanRef]]:
    """Build an IKAM storage artifact + fragments for a Petri net."""

    require_unique_place_ids(places)
    require_unique_transition_ids(transitions)
    require_transition_place_refs(transitions, [p.place_id for p in places])
    require_non_negative_marking(marking)

    place_fragments = [
        StoredFragment.from_bytes(canonicalize_petri_net_place_json(p), mime_type="application/json")
        for p in places
    ]
    transition_fragments = [
        StoredFragment.from_bytes(canonicalize_petri_net_transition_json(t), mime_type="application/json")
        for t in transitions
    ]
    arc_fragments = [
        StoredFragment.from_bytes(canonicalize_petri_net_arc_json(a), mime_type="application/json")
        for a in (arcs or [])
    ]
    marking_fragment = StoredFragment.from_bytes(
        canonicalize_petri_net_marking_json(marking),
        mime_type="application/json",
    )
    section_fragments = [
        StoredFragment.from_bytes(canonicalize_petri_net_section_json(s), mime_type="application/json")
        for s in (sections or [])
    ]

    envelope = PetriNetEnvelope(
        project_id=project_id,
        scope_id=scope_id,
        title=title,
        goal=goal,
        place_fragment_ids=[f.id for f in place_fragments],
        transition_fragment_ids=[f.id for f in transition_fragments],
        arc_fragment_ids=[f.id for f in arc_fragments],
        initial_marking_fragment_id=marking_fragment.id,
        index_fragment_id=index_fragment.id if index_fragment else None,
    )
    envelope_bytes = canonicalize_petri_net_envelope_json(envelope)
    envelope_fragment = StoredFragment.from_bytes(envelope_bytes, mime_type="application/json")

    artifact_created_at = created_at or dt.datetime.now(dt.timezone.utc)
    artifact_id = petri_net_artifact_uuid(envelope_bytes)
    fragments: List[StoredFragment] = [
        envelope_fragment,
        *place_fragments,
        *transition_fragments,
        *arc_fragments,
        marking_fragment,
        *section_fragments,
    ]
    if index_fragment is not None:
        fragments.append(index_fragment)

    artifact = Artifact(
        id=str(artifact_id),
        kind="file",
        title=title,
        root_fragment_id=envelope_fragment.id,
        created_at=artifact_created_at,
    )

    plan_ref = PlanRef(artifact_id=str(artifact_id), fragment_id=envelope_fragment.id)
    transition_refs = {
        t.transition_id: PlanRef(artifact_id=str(artifact_id), fragment_id=f.id)
        for t, f in zip(transitions, transition_fragments)
    }

    return artifact, fragments, plan_ref, transition_refs


def petri_run_to_ikam_artifact(
    *,
    envelope: PetriNetRunEnvelope,
    firings: List[PetriNetRunFiring],
    created_at: Optional[dt.datetime] = None,
) -> Tuple[Artifact, List[StoredFragment], PlanRef]:
    """Build an IKAM storage artifact + fragments for a PetriNetRun."""

    firing_fragments = [
        StoredFragment.from_bytes(canonicalize_petri_run_firing_json(f), mime_type="application/json")
        for f in firings
    ]

    envelope_model = PetriNetRunEnvelope.model_validate(envelope)
    envelope_model.firing_fragment_ids = [f.id for f in firing_fragments]

    envelope_bytes = canonicalize_petri_run_envelope_json(envelope_model)
    envelope_fragment = StoredFragment.from_bytes(envelope_bytes, mime_type="application/json")

    artifact_created_at = created_at or dt.datetime.now(dt.timezone.utc)
    artifact_id = petri_run_artifact_uuid(envelope_bytes)
    fragments = [envelope_fragment, *firing_fragments]

    artifact = Artifact(
        id=str(artifact_id),
        kind="file",
        title=f"PetriNetRun {envelope_model.run_id}",
        root_fragment_id=envelope_fragment.id,
        created_at=artifact_created_at,
    )

    plan_run_ref = PlanRef(artifact_id=str(artifact_id), fragment_id=envelope_fragment.id)
    return artifact, fragments, plan_run_ref


def reconstruct_petri_net_from_fragments(
    fragments: List[Fragment],
) -> Dict[str, Any]:
    """Reconstruct a Petri net view payload from storage fragments."""

    envelope_fragment: Optional[Fragment] = None
    for frag in fragments:
        try:
            PetriNetEnvelope.model_validate_json(frag.bytes)
            envelope_fragment = frag
            break
        except Exception:
            continue

    if envelope_fragment is None:
        return {}

    envelope = PetriNetEnvelope.model_validate_json(envelope_fragment.bytes)
    fragments_by_id: Dict[str, Fragment] = {f.id: f for f in fragments}

    places: List[Dict[str, Any]] = []
    for frag_id in envelope.place_fragment_ids:
        frag = fragments_by_id.get(frag_id)
        if frag is None:
            continue
        try:
            place = PetriNetPlace.model_validate_json(frag.bytes)
        except Exception:
            continue
        places.append(place.model_dump(by_alias=True))

    transitions: List[Dict[str, Any]] = []
    for frag_id in envelope.transition_fragment_ids:
        frag = fragments_by_id.get(frag_id)
        if frag is None:
            continue
        try:
            transition = PetriNetTransition.model_validate_json(frag.bytes)
        except Exception:
            continue
        transitions.append(transition.model_dump(by_alias=True))

    arcs: List[Dict[str, Any]] = []
    for frag_id in envelope.arc_fragment_ids:
        frag = fragments_by_id.get(frag_id)
        if frag is None:
            continue
        try:
            arc = PetriNetArc.model_validate_json(frag.bytes)
        except Exception:
            continue
        arcs.append(arc.model_dump(by_alias=True))

    marking: Optional[Dict[str, Any]] = None
    marking_frag = fragments_by_id.get(envelope.initial_marking_fragment_id)
    if marking_frag is not None:
        try:
            marking_model = PetriNetMarking.model_validate_json(marking_frag.bytes)
            marking = marking_model.model_dump(by_alias=True)
        except Exception:
            marking = None

    sections: List[Dict[str, Any]] = []
    for frag in fragments:
        try:
            section = PetriNetSection.model_validate_json(frag.bytes)
        except Exception:
            continue
        sections.append(section.model_dump(by_alias=True))
    sections.sort(key=lambda s: ",".join(s.get("transition_ids") or []))

    index_payload: Optional[Dict[str, Any]] = None
    if envelope.index_fragment_id:
        index_frag = fragments_by_id.get(envelope.index_fragment_id)
        if index_frag is not None:
            try:
                index_payload = json.loads(index_frag.bytes)
            except Exception:
                index_payload = None

    return {
        "envelope": envelope.model_dump(by_alias=True),
        "envelope_fragment_id": envelope_fragment.id,
        "places": places,
        "transitions": transitions,
        "arcs": arcs,
        "marking": marking,
        "sections": sections,
        "index": index_payload,
    }
