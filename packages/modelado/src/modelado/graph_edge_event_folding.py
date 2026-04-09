from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Iterable

from modelado.graph_edge_event_log import GraphEdgeEvent, compute_edge_identity_key


@dataclass(frozen=True)
class EffectiveEdge:
    edge_key: str
    edge_label: str
    out_id: str
    in_id: str
    properties: dict[str, Any]
    t: int
    last_event_id: int


def fold_effective_edges(events: Iterable[GraphEdgeEvent]) -> dict[str, EffectiveEdge]:
    """Fold an append-only edge-event log into the effective edge state.

    Events must be provided in deterministic replay order (ascending `id`).
    Folding is "last write wins" per effective edge identity.
    """

    effective: dict[str, EffectiveEdge] = {}

    for e in events:
        edge_key = compute_edge_identity_key(
            edge_label=e.edge_label,
            out_id=e.out_id,
            in_id=e.in_id,
            properties=e.properties,
        )

        if e.op == "delete":
            if is_subtree_graph_delta_delete(e):
                delete_matching_subtree_edges(effective, e)
                continue
            effective.pop(edge_key, None)
            continue

        effective[edge_key] = EffectiveEdge(
            edge_key=edge_key,
            edge_label=e.edge_label,
            out_id=e.out_id,
            in_id=e.in_id,
            properties=dict(e.properties or {}),
            t=int(e.t),
            last_event_id=int(e.id),
        )

    return effective


def is_subtree_graph_delta_delete(event: GraphEdgeEvent) -> bool:
    return (
        event.op == "delete"
        and event.edge_label == "graph:value_at"
        and event.properties.get("graphDeltaExtent") == "subtree"
        and isinstance(event.properties.get("graphDeltaHandle"), str)
        and isinstance(event.properties.get("graphDeltaPath"), list)
    )


def delete_matching_subtree_edges(effective: dict[str, Any], event: GraphEdgeEvent) -> None:
    handle = event.properties.get("graphDeltaHandle")
    path = event.properties.get("graphDeltaPath")
    if not isinstance(handle, str) or not isinstance(path, list):
        return
    prefix = tuple(path)
    keys_to_delete = [
        edge_key
        for edge_key, edge in effective.items()
        if _matches_graph_delta_subtree(edge, handle=handle, prefix=prefix)
    ]
    for edge_key in keys_to_delete:
        effective.pop(edge_key, None)


def _matches_graph_delta_subtree(edge: Any, *, handle: str, prefix: tuple[Any, ...]) -> bool:
    properties = getattr(edge, "properties", None)
    if not isinstance(properties, dict):
        return False
    if properties.get("graphDeltaHandle") != handle:
        return False
    path = properties.get("graphDeltaPath")
    if not isinstance(path, list):
        return False
    return tuple(path[: len(prefix)]) == prefix
