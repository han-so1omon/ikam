"""Compile authored workflows into richer Petri workflows."""

from __future__ import annotations

from interacciones.schemas import RichPetriArc, RichPetriPlace, RichPetriTransition, RichPetriWorkflow, SourceWorkflowStorageMode, SourceWorkflowStoragePolicy, WorkflowDefinition, WorkflowNode
from interacciones.schemas.workflows import SUPPORTED_WORKFLOW_NODE_KINDS

from .validator import validate_workflow_definition


def compile_workflow_definition(value: WorkflowDefinition | dict[str, object]) -> RichPetriWorkflow:
    workflow = validate_workflow_definition(value)
    places, arcs = _build_places_and_arcs(workflow)
    transitions = tuple(_compile_transition(node) for node in workflow.nodes)
    return RichPetriWorkflow(
        workflow_id=workflow.workflow_id,
        version=workflow.version,
        places=list(places),
        transitions=list(transitions),
        arcs=list(arcs),
        publish=list(workflow.publish),
        source_workflow_storage=SourceWorkflowStoragePolicy(mode=SourceWorkflowStorageMode.DEFAULT_ON),
        source_workflow_definition=workflow,
    )


def _build_places_and_arcs(workflow: WorkflowDefinition) -> tuple[tuple[RichPetriPlace, ...], tuple[RichPetriArc, ...]]:
    incoming: dict[str, list[str]] = {node.node_id: [] for node in workflow.nodes}
    outgoing: dict[str, list[str]] = {node.node_id: [] for node in workflow.nodes}
    for link in workflow.links:
        outgoing[link.source].append(link.target)
        incoming[link.target].append(link.source)

    places: list[RichPetriPlace] = []
    arcs: list[RichPetriArc] = []
    seen_places: set[str] = set()

    for node in workflow.nodes:
        transition_id = _transition_id(node.node_id)
        if not incoming[node.node_id]:
            start_place = _start_place_id(node.node_id)
            _append_place(places, seen_places, start_place)
            arcs.append(_place_to_transition(start_place, transition_id))
        for source in incoming[node.node_id]:
            link_place = _link_place_id(source, node.node_id)
            _append_place(places, seen_places, link_place)
            arcs.append(_place_to_transition(link_place, transition_id))
        for target in outgoing[node.node_id]:
            link_place = _link_place_id(node.node_id, target)
            _append_place(places, seen_places, link_place)
            arcs.append(_transition_to_place(transition_id, link_place))
        if not outgoing[node.node_id]:
            end_place = _end_place_id(node.node_id)
            _append_place(places, seen_places, end_place)
            arcs.append(_transition_to_place(transition_id, end_place))

    return tuple(places), tuple(arcs)


def _compile_transition(node: WorkflowNode) -> RichPetriTransition:
    _validate_supported_node_kind(node.kind)
    transition_policy = dict(node.policy)
    if node.operator_selection and "operator_selection" not in transition_policy:
        transition_policy["operator_selection"] = dict(node.operator_selection)
    if node.executor_selection and "executor_selection" not in transition_policy:
        transition_policy["executor_selection"] = dict(node.executor_selection)
    return RichPetriTransition(
        transition_id=_transition_id(node.node_id),
        label=node.node_id,
        capability=node.capability,
        policy=transition_policy,
        constraints=node.constraints,
        validators=node.validators,
        resolution_mode=node.resolution_mode,
        direct_executor_ref=node.direct_executor_ref,
        trace_policy=None,
        approval_hint=_approval_hint(node.kind),
        checkpoint_hint=_checkpoint_hint(node.kind),
    )


def _approval_hint(node_kind: str) -> dict[str, str]:
    if node_kind in {"request_approval", "wait_for_approval"}:
        return {"node_kind": node_kind}
    return {}


def _checkpoint_hint(node_kind: str) -> dict[str, str]:
    if node_kind in {
        "wait_for_result",
        "checkpoint",
        "emit_event",
        "await_event",
        "route",
        "complete",
        "fail",
        "emit_mcp_call",
        "await_mcp_response",
        "emit_acp_message",
        "await_acp_message",
    }:
        return {"node_kind": node_kind}
    return {}


def _validate_supported_node_kind(node_kind: str) -> None:
    if node_kind not in SUPPORTED_WORKFLOW_NODE_KINDS:
        raise ValueError(f"workflow node kind '{node_kind}' is not supported by rich Petri compilation")


def _append_place(places: list[RichPetriPlace], seen_places: set[str], place_id: str) -> None:
    if place_id in seen_places:
        return
    seen_places.add(place_id)
    places.append(RichPetriPlace(place_id=place_id, label=place_id))


def _place_to_transition(place_id: str, transition_id: str) -> RichPetriArc:
    return RichPetriArc(
        source_kind="place",
        source_id=place_id,
        target_kind="transition",
        target_id=transition_id,
    )


def _transition_to_place(transition_id: str, place_id: str) -> RichPetriArc:
    return RichPetriArc(
        source_kind="transition",
        source_id=transition_id,
        target_kind="place",
        target_id=place_id,
    )


def _transition_id(node_id: str) -> str:
    return f"transition:{node_id}"


def _start_place_id(node_id: str) -> str:
    return f"place:start:{node_id}"


def _end_place_id(node_id: str) -> str:
    return f"place:end:{node_id}"


def _link_place_id(source: str, target: str) -> str:
    return f"place:link:{source}:{target}"
