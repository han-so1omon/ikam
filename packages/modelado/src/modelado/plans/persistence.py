"""Plan persistence — awaiting repository rewrite.

All persist_* functions previously called insert_artifact() and insert_derivation()
from ikam_graph_repository, which have been deleted as part of the legacy write path
cleanup (Phase 4b). These functions will be reimplemented once the new fragment store
write path lands (Task 4b.3+).

Functions that only use insert_fragment() (CAS fragment writes) remain functional.
"""
from __future__ import annotations

import datetime as dt
from typing import Any, Dict, List, Optional, Tuple, Union

from ikam.graph import StoredFragment

from modelado.ikam_graph_repository import insert_fragment

from .artifacts import (
    plan_amendment_to_ikam_artifact,
    petri_net_to_ikam_artifact,
    petri_run_to_ikam_artifact,
    plan_to_ikam_artifact,
)
from .schema import (
    ArtifactRef,
    Plan,
    PlanAmendment,
    PlanRef,
    PetriNetMarking,
    PetriNetPlace,
    PetriNetTransition,
    PetriNetArc,
)
from .verification import PlanVerificationReport

try:
    from modelado.graph.ir import LoweredExecutableGraph
except ImportError:  # pragma: no cover - graph compiler lands incrementally
    LoweredExecutableGraph = None  # type: ignore[assignment]


def graph_compilation_fragments(compilation: "LoweredExecutableGraph") -> list[Any]:
    """Expose lowered IKAM-native fragments for future persistence wiring."""
    return compilation.fragments


def persist_plan(
    cx: Any,
    plan: Union[Plan, Dict[str, Any]],
    *,
    created_at: Optional[dt.datetime] = None,
    author_id: Optional[str] = None,
) -> PlanRef:
    """Persist a plan as IKAM fragments (artifact write path pending rewrite)."""
    raise NotImplementedError(
        "persist_plan: insert_artifact() deleted in Phase 4b. "
        "Awaiting new artifact write path in Task 4b.3."
    )


def persist_petri_net(
    cx: Any,
    *,
    project_id: str,
    scope_id: str,
    title: str,
    goal: str,
    places: List[PetriNetPlace],
    transitions: List[PetriNetTransition],
    arcs: Optional[List[PetriNetArc]],
    marking: PetriNetMarking,
    sections: Optional[List[Any]] = None,
    index_fragment: Optional[Fragment] = None,
    created_at: Optional[dt.datetime] = None,
    author_id: Optional[str] = None,
) -> PlanRef:
    """Persist a Petri net using IKAM Layer 0 IR primitives."""
    import uuid
    import json
    from ikam.ir.core import StructuredDataIR, ExpressionIR, PropositionIR, OpAST, OpType, EvidenceRef
    from modelado.ikam_graph_repository import store_fragment
    from ikam.fragments import Fragment as DomainFragment
    from ikam.graph import _cas_hex
    
    artifact_id = str(uuid.uuid4())
    
    def _store_ir(ir_model: Any) -> str:
        ir_dict = ir_model.model_dump()
        cas_id = _cas_hex(json.dumps(ir_dict, sort_keys=True).encode("utf-8"))
        domain_frag = DomainFragment(
            cas_id=cas_id,
            value=ir_dict,
            mime_type=f"application/ikam-ir+{ir_model.__class__.__name__.lower()}+json"
        )
        store_fragment(cx, domain_frag, project_id=project_id)
        return cas_id

    # Translate places
    place_cas_ids = []
    for p in places:
        ir_p = StructuredDataIR(
            artifact_id=artifact_id,
            profile="petri_place",
            data=p.model_dump()
        )
        place_cas_ids.append(_store_ir(ir_p))
        
    # Translate transitions to ExpressionIR
    transition_cas_ids = []
    for t in transitions:
        ir_t = ExpressionIR(
            artifact_id=artifact_id,
            ast=OpAST(
                op_type=OpType.REF,
                params={"operation_ref": t.operation_ref, "transition_data": t.model_dump()}
            )
        )
        transition_cas_ids.append(_store_ir(ir_t))
        
    # Arcs as PropositionIR
    arc_cas_ids = []
    for a in (arcs or []):
        ir_a = PropositionIR(
            artifact_id=artifact_id,
            profile="petri_arc",
            statement=a.model_dump(),
            evidence_refs=[EvidenceRef(fragment_id=place_cas_ids[0] if place_cas_ids else "dummy_place")] 
        )
        arc_cas_ids.append(_store_ir(ir_a))
        
    # Marking
    marking_ir = StructuredDataIR(
        artifact_id=artifact_id,
        profile="petri_marking",
        data=marking.model_dump()
    )
    marking_cas_id = _store_ir(marking_ir)
    
    # Sections
    section_cas_ids = []
    for s in (sections or []):
        ir_s = StructuredDataIR(
            artifact_id=artifact_id,
            profile="petri_section",
            data=s.model_dump() if hasattr(s, "model_dump") else s
        )
        section_cas_ids.append(_store_ir(ir_s))
        
    # Envelope
    envelope_data = {
        "project_id": project_id,
        "scope_id": scope_id,
        "title": title,
        "goal": goal,
        "place_cas_ids": place_cas_ids,
        "transition_cas_ids": transition_cas_ids,
        "arc_cas_ids": arc_cas_ids,
        "marking_cas_id": marking_cas_id,
        "section_cas_ids": section_cas_ids
    }
    envelope_ir = StructuredDataIR(
        artifact_id=artifact_id,
        profile="petri_net",
        data=envelope_data
    )
    envelope_cas_id = _store_ir(envelope_ir)
    
    from modelado.registry import get_shared_registry_manager

    get_shared_registry_manager().append_put(
        cx,
        namespace="petri_net_runnables",
        key=envelope_cas_id,
        value={
            "type": "subgraph_ref",
            "head_fragment_id": envelope_cas_id,
            "title": title,
            "goal": goal,
            "registered_at": dt.datetime.now(dt.timezone.utc).isoformat(),
            "project_id": project_id
        }
    )
    
    return PlanRef(artifact_id=artifact_id, fragment_id=envelope_cas_id)


def persist_petri_net_run(
    cx: Any,
    *,
    envelope: Any,
    firings: List[Any],
    created_at: Optional[dt.datetime] = None,
    author_id: Optional[str] = None,
) -> PlanRef:
    """Persist a petri net run (artifact write path pending rewrite)."""
    raise NotImplementedError(
        "persist_petri_net_run: insert_artifact() deleted in Phase 4b. "
        "Awaiting new artifact write path in Task 4b.3."
    )


def create_root_plan_net(
    cx: Any,
    *,
    project_id: str,
    scope_id: str,
    title: str,
    goal: str,
    author_id: Optional[str] = None,
) -> PlanRef:
    """Create a minimal Petri net (artifact write path pending rewrite)."""
    raise NotImplementedError(
        "create_root_plan_net: depends on persist_petri_net. "
        "Awaiting new artifact write path in Task 4b.3."
    )


def persist_plan_amendment(
    cx: Any,
    amendment: Union[PlanAmendment, Dict[str, Any]],
    *,
    created_at: Optional[dt.datetime] = None,
    author_id: Optional[str] = None,
) -> ArtifactRef:
    """Persist a plan amendment (artifact write path pending rewrite)."""
    raise NotImplementedError(
        "persist_plan_amendment: insert_artifact() deleted in Phase 4b. "
        "Awaiting new artifact write path in Task 4b.3."
    )


def persist_plan_verification_report(
    cx: Any,
    report: Union[PlanVerificationReport, Dict[str, Any]],
    *,
    created_at: Optional[dt.datetime] = None,
    author_id: Optional[str] = None,
) -> ArtifactRef:
    """Persist a plan verification report (artifact write path pending rewrite)."""
    raise NotImplementedError(
        "persist_plan_verification_report: insert_artifact() deleted in Phase 4b. "
        "Awaiting new artifact write path in Task 4b.3."
    )


def persist_plan_amendment_application(
    cx: Any,
    *,
    base_plan: Union[Plan, Dict[str, Any]],
    amendment: Union[PlanAmendment, Dict[str, Any]],
    created_at: Optional[dt.datetime] = None,
    author_id: Optional[str] = None,
) -> Tuple[ArtifactRef, PlanRef, Optional[str]]:
    """Persist amendment application with derivation (artifact write path pending rewrite)."""
    raise NotImplementedError(
        "persist_plan_amendment_application: insert_artifact(), insert_derivation(), "
        "get_delta_chain_length_in_project() all deleted in Phase 4b. "
        "Awaiting new artifact write path in Task 4b.3."
    )


def persist_plan_in_scope(
    cx: Any,
    plan: Union[Plan, Dict[str, Any]],
    *,
    created_at: Optional[dt.datetime] = None,
    author_id: Optional[str] = None,
) -> PlanRef:
    """Persist a plan and associate with scope (artifact write path pending rewrite)."""
    raise NotImplementedError(
        "persist_plan_in_scope: depends on persist_plan. "
        "Awaiting new artifact write path in Task 4b.3."
    )
