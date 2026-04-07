from __future__ import annotations

from typing import Iterable, Optional

from psycopg import Connection

from modelado.graph_edge_event_log import GraphEdgeEvent, append_graph_edge_event


def append_plan_run_of_edge(
    cx: Connection,
    *,
    project_id: str,
    plan_artifact_id: str,
    plan_fragment_id: str,
    plan_run_artifact_id: str,
    run_id: str,
) -> Optional[GraphEdgeEvent]:
    """Record PlanRun → Plan linkage in the graph edge event log."""

    return append_graph_edge_event(
        cx,
        project_id=project_id,
        op="upsert",
        edge_label="derivation:plan_run_of",
        out_id=plan_run_artifact_id,
        in_id=plan_artifact_id,
        properties={
            "plan_fragment_id": plan_fragment_id,
            "run_id": run_id,
        },
    )


def append_fired_transition_edge(
    cx: Connection,
    *,
    project_id: str,
    plan_artifact_id: str,
    plan_run_artifact_id: str,
    transition_fragment_id: str,
    transition_id: str,
    firing_fragment_id: str,
) -> Optional[GraphEdgeEvent]:
    """Record PlanRun firing of a transition (PlanRun → Plan)."""

    return append_graph_edge_event(
        cx,
        project_id=project_id,
        op="upsert",
        edge_label="derivation:fired_transition",
        out_id=plan_run_artifact_id,
        in_id=plan_artifact_id,
        properties={
            "transition_fragment_id": transition_fragment_id,
            "transition_id": transition_id,
            "firing_fragment_id": firing_fragment_id,
        },
    )


def append_derived_from_edges(
    cx: Connection,
    *,
    project_id: str,
    plan_artifact_id: str,
    run_id: str,
    firing_fragment_id: str,
    output_artifact_ids: Iterable[str],
    input_artifact_ids: Iterable[str],
    transition_fragment_id: Optional[str] = None,
) -> list[GraphEdgeEvent]:
    """Record derivation edges for artifacts produced during a PlanRun."""

    events: list[GraphEdgeEvent] = []
    props: dict[str, str] = {
        "run_id": run_id,
        "plan_artifact_id": plan_artifact_id,
        "firing_fragment_id": firing_fragment_id,
    }
    if transition_fragment_id:
        props["transition_fragment_id"] = transition_fragment_id

    for out_id in output_artifact_ids:
        for in_id in input_artifact_ids:
            event = append_graph_edge_event(
                cx,
                project_id=project_id,
                op="upsert",
                edge_label="derivation:derived_from",
                out_id=str(out_id),
                in_id=str(in_id),
                properties=props,
            )
            if event is not None:
                events.append(event)

    return events
