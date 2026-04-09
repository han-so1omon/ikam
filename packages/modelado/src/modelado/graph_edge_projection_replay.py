from __future__ import annotations

from dataclasses import dataclass
from typing import Iterable, Optional

from psycopg import Connection

from modelado.environment_scope import scope_ref_from_qualifiers
from modelado.graph_edge_event_folding import EffectiveEdge, delete_matching_subtree_edges, is_subtree_graph_delta_delete
from modelado.graph_edge_event_log import GraphEdgeEvent, compute_edge_identity_key, list_graph_edge_events


@dataclass(frozen=True)
class ProjectionPolicy:
    """Policy metadata for deterministic projection/replay."""

    edge_label_prefix: Optional[str] = "knowledge:"
    policy_version: str = "v1"
    ref: Optional[str] = None
    env_type: Optional[str] = None
    env_id: Optional[str] = None
    pipeline_id: Optional[str] = None
    pipeline_run_id: Optional[str] = None


@dataclass(frozen=True)
class ReplaySnapshot:
    """Result of replaying edge events into an effective graph snapshot."""

    effective_edges: dict[str, EffectiveEdge]
    last_event_id: int
    policy: ProjectionPolicy


def replay_effective_edges(
    events: Iterable[GraphEdgeEvent],
    *,
    policy: ProjectionPolicy,
    base: Optional[dict[str, EffectiveEdge]] = None,
) -> ReplaySnapshot:
    """Replay edge events into an effective graph snapshot deterministically.

    This is pure and deterministic; it only depends on the edge-event log and policy.
    """

    effective: dict[str, EffectiveEdge] = dict(base or {})
    last_event_id = 0

    for event in events:
        last_event_id = max(last_event_id, int(event.id))
        if policy.edge_label_prefix and not str(event.edge_label or "").startswith(policy.edge_label_prefix):
            continue

        props = dict(event.properties or {})
        event_ref = scope_ref_from_qualifiers(props)
        if policy.ref is not None and event_ref != policy.ref:
            continue
        if policy.env_type is not None and props.get("envType") != policy.env_type:
            continue
        if policy.env_id is not None and props.get("envId") != policy.env_id:
            continue
        if policy.pipeline_id is not None and props.get("pipelineId") != policy.pipeline_id:
            continue
        if policy.pipeline_run_id is not None and props.get("pipelineRunId") != policy.pipeline_run_id:
            continue

        edge_key = compute_edge_identity_key(
            edge_label=event.edge_label,
            out_id=event.out_id,
            in_id=event.in_id,
            properties=props,
        )

        if event.op == "delete":
            if is_subtree_graph_delta_delete(event):
                delete_matching_subtree_edges(effective, event)
                continue
            effective.pop(edge_key, None)
            continue

        effective[edge_key] = EffectiveEdge(
            edge_key=edge_key,
            edge_label=event.edge_label,
            out_id=event.out_id,
            in_id=event.in_id,
            properties=props,
            t=int(event.t),
            last_event_id=int(event.id),
        )

    return ReplaySnapshot(effective_edges=effective, last_event_id=last_event_id, policy=policy)


def replay_effective_edges_from_log(
    cx: Connection,
    *,
    project_id: str,
    policy: ProjectionPolicy,
    after_id: int = 0,
    batch_size: int = 500,
    base: Optional[dict[str, EffectiveEdge]] = None,
) -> ReplaySnapshot:
    """Replay edge events from the authoritative log in deterministic order.

    The edge-event log is the sole source of truth for the effective graph snapshot.
    """

    effective: dict[str, EffectiveEdge] = dict(base or {})
    last_processed = int(after_id)
    cursor = int(after_id)

    while True:
        events = list_graph_edge_events(
            cx,
            project_id=project_id,
            after_id=cursor,
            limit=int(batch_size),
        )
        if not events:
            break

        snapshot = replay_effective_edges(events, policy=policy, base=effective)
        effective = snapshot.effective_edges
        last_processed = max(last_processed, snapshot.last_event_id)
        cursor = max(cursor, snapshot.last_event_id)

    return ReplaySnapshot(effective_edges=effective, last_event_id=last_processed, policy=policy)
