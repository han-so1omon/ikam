from __future__ import annotations

from dataclasses import dataclass
from typing import Iterable, Optional

from modelado.hugegraph_client import HugeGraphClient


_VLBL_CACHE: dict[tuple[str, str, str, str], int] = {}


class GraphTraversalLimitExceeded(RuntimeError):
    pass


@dataclass(frozen=True)
class DerivationEdge:
    out_id: str
    in_id: str
    edge_key: str
    edge_label: str
    t: int
    derivation_id: str | None
    derivation_type: str | None


def _parse_edge(e: dict) -> DerivationEdge:
    props = dict(e.get("properties") or {})
    return DerivationEdge(
        out_id=str(e.get("outV")),
        in_id=str(e.get("inV")),
        edge_key=str(props.get("edge_key") or ""),
        edge_label=str(props.get("edge_label") or ""),
        t=int(props.get("t") or 0),
        derivation_id=(str(props.get("derivation_id")) if props.get("derivation_id") is not None else None),
        derivation_type=(
            str(props.get("derivation_type")) if props.get("derivation_type") is not None else None
        ),
    )


def _vlabel_id(client: HugeGraphClient, label: str) -> int:
    key = (client.base_url, client.graph, client.graphspace, label)
    cached = _VLBL_CACHE.get(key)
    if cached is not None:
        return cached
    vlabel = client.schema_get("vertexlabels", label)
    if not isinstance(vlabel, dict) or "id" not in vlabel:
        raise RuntimeError(f"HugeGraph schema missing {label} vertex label id")
    v_id = int(vlabel["id"])
    _VLBL_CACHE[key] = v_id
    return v_id


def _to_vertex_id(client: HugeGraphClient, id: str, label: str) -> str:
    vlabel_id = _vlabel_id(client, label)
    return f"{vlabel_id}:{id}"


def _from_vertex_id(client: HugeGraphClient, vertex_id: str, label: str) -> str:
    prefix = f"{_vlabel_id(client, label)}:"
    if vertex_id.startswith(prefix):
        return vertex_id[len(prefix) :]
    return vertex_id


def fetch_incoming_edges(
    client: HugeGraphClient,
    ids: Iterable[str],
    *,
    label: str = "Derivation",
    vlabel: str = "Artifact",
    project_id: Optional[str] = None,
    limit_per_vertex: int = 10_000,
) -> list[DerivationEdge]:
    out: list[DerivationEdge] = []
    for id in ids:
        vertex_id = _to_vertex_id(client, id, vlabel)
        props = {"project_id": project_id} if project_id else None
        raw = client.list_edges(
            vertex_id=vertex_id,
            direction="IN",
            label=label,
            properties=props,
            limit=limit_per_vertex,
        )
        for e in (raw.get("edges") or []):
            edge = _parse_edge(e)
            out.append(
                DerivationEdge(
                    out_id=_from_vertex_id(client, edge.out_id, vlabel),
                    in_id=_from_vertex_id(client, edge.in_id, vlabel),
                    edge_key=edge.edge_key,
                    edge_label=edge.edge_label,
                    t=edge.t,
                    derivation_id=edge.derivation_id,
                    derivation_type=edge.derivation_type,
                )
            )
    return out


def fetch_outgoing_edges(
    client: HugeGraphClient,
    ids: Iterable[str],
    *,
    label: str = "Derivation",
    vlabel: str = "Artifact",
    project_id: Optional[str] = None,
    limit_per_vertex: int = 10_000,
) -> list[DerivationEdge]:
    out: list[DerivationEdge] = []
    for id in ids:
        vertex_id = _to_vertex_id(client, id, vlabel)
        props = {"project_id": project_id} if project_id else None
        raw = client.list_edges(
            vertex_id=vertex_id,
            direction="OUT",
            label=label,
            properties=props,
            limit=limit_per_vertex,
        )
        for e in (raw.get("edges") or []):
            edge = _parse_edge(e)
            out.append(
                DerivationEdge(
                    out_id=_from_vertex_id(client, edge.out_id, vlabel),
                    in_id=_from_vertex_id(client, edge.in_id, vlabel),
                    edge_key=edge.edge_key,
                    edge_label=edge.edge_label,
                    t=edge.t,
                    derivation_id=edge.derivation_id,
                    derivation_type=edge.derivation_type,
                )
            )
    return out
