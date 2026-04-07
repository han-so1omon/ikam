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
