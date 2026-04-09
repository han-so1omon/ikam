from __future__ import annotations

import asyncio
from dataclasses import dataclass
import json
from typing import Any, Mapping, Optional

import psycopg

from modelado.graph_edge_event_folding import is_subtree_graph_delta_delete
from modelado.graph_edge_event_log import GraphEdgeEvent, compute_edge_identity_key, list_graph_edge_events
from modelado.hugegraph_client import HugeGraphClient, HugeGraphError


_ARTIFACT_VLABEL = "Artifact"
_FRAGMENT_VLABEL = "Fragment"
_PROPOSITION_VLABEL = "Proposition"
_EXPRESSION_VLABEL = "Expression"
_STRUCTURED_DATA_VLABEL = "StructuredData"

_DERIVATION_ELABEL = "Derivation"
_VALUE_AT_ELABEL = "value-at"
_ROOT_ELABEL = "root"
_CONNECTION_ELABEL = "connection"
_CALCULATES_ELABEL = "calculates"
_FEEDS_ELABEL = "feeds"
_SEQUENCED_IN_ELABEL = "sequenced-in"
_SUPPORTS_ELABEL = "supports"
_REBUTS_ELABEL = "rebuts"
_UNDERCUTS_ELABEL = "undercuts"


def _id_strategy(client: HugeGraphClient, label: str) -> str:
    try:
        vlabel = client.schema_get("vertexlabels", label)
        if isinstance(vlabel, dict) and "id_strategy" in vlabel:
            return str(vlabel.get("id_strategy") or "").upper()
        if isinstance(vlabel, dict) and "vertexlabel" in vlabel and isinstance(vlabel["vertexlabel"], dict):
            return str(vlabel["vertexlabel"].get("id_strategy") or "").upper()
    except Exception:
        pass
    return ""


def _vlabel_id(client: HugeGraphClient, label: str) -> int | None:
    try:
        vlabel = client.schema_get("vertexlabels", label)
        if isinstance(vlabel, dict) and "id" in vlabel:
            vlabel_id = vlabel.get("id")
            if vlabel_id is not None:
                return int(str(vlabel_id))
        if isinstance(vlabel, dict) and "vertexlabel" in vlabel and isinstance(vlabel["vertexlabel"], dict):
            if "id" in vlabel["vertexlabel"]:
                vlabel_id = vlabel["vertexlabel"].get("id")
                if vlabel_id is not None:
                    return int(str(vlabel_id))
    except Exception:
        pass
    return None


def ensure_ikam_projection_schema(client: HugeGraphClient) -> None:
    """Create the HugeGraph schema for Full IR Coverage (M12)."""

    def ensure_property_key(name: str, data_type: str) -> None:
        try:
            client.schema_get("propertykeys", name)
            return
        except Exception:
            pass
        try:
            client.schema_create(
                "propertykeys",
                {"name": name, "data_type": data_type, "cardinality": "SINGLE"},
            )
        except HugeGraphError as e:
            if e.status in {400, 409}:
                return
            raise

    for key, dtype in [
        ("id", "TEXT"),
        ("project_id", "TEXT"),
        ("edge_key", "TEXT"),
        ("edge_label", "TEXT"),
        ("graphDeltaHandle", "TEXT"),
        ("graphDeltaPath", "TEXT"),
        ("envType", "TEXT"),
        ("envId", "TEXT"),
        ("t", "LONG"),
        ("derivation_id", "TEXT"),
        ("derivation_type", "TEXT"),
        ("type", "TEXT"),
        ("mime_type", "TEXT"),
        ("salience", "DOUBLE"),
        ("confidence", "DOUBLE"),
    ]:
        ensure_property_key(key, dtype)

    def ensure_vertex_label(name: str, primary_keys: list[str], nullable_keys: list[str]) -> None:
        try:
            client.schema_get("vertexlabels", name)
        except Exception:
            client.schema_create(
                "vertexlabels",
                {
                    "name": name,
                    "id_strategy": "PRIMARY_KEY",
                    "properties": ["id", "project_id", "type", "mime_type", "salience", "confidence"],
                    "primary_keys": primary_keys,
                    "nullable_keys": nullable_keys + ["type", "mime_type", "salience", "confidence"],
                    "enable_label_index": True,
                },
            )

    ensure_vertex_label(_ARTIFACT_VLABEL, ["id"], ["project_id"])
    ensure_vertex_label(_FRAGMENT_VLABEL, ["id"], ["project_id"])
    ensure_vertex_label(_PROPOSITION_VLABEL, ["id"], ["project_id"])
    ensure_vertex_label(_EXPRESSION_VLABEL, ["id"], ["project_id"])
    ensure_vertex_label(_STRUCTURED_DATA_VLABEL, ["id"], ["project_id"])

    def ensure_edge_label(name: str, source: str, target: str) -> None:
        try:
            client.schema_get("edgelabels", name)
        except Exception:
            client.schema_create(
                "edgelabels",
                {
                    "name": name,
                    "source_label": source,
                    "target_label": target,
                    "frequency": "MULTIPLE",
                    "properties": [
                        "edge_key",
                        "project_id",
                        "edge_label",
                        "graphDeltaHandle",
                        "graphDeltaPath",
                        "envType",
                        "envId",
                        "t",
                        "derivation_id",
                        "derivation_type",
                        "confidence",
                    ],
                    "sort_keys": ["edge_key"],
                    "nullable_keys": [
                        "envType",
                        "envId",
                        "graphDeltaHandle",
                        "graphDeltaPath",
                        "derivation_id",
                        "derivation_type",
                        "confidence",
                    ],
                },
            )

    # Core Derivations and graph-native addressed content
    ensure_edge_label(_DERIVATION_ELABEL, _ARTIFACT_VLABEL, _ARTIFACT_VLABEL)
    ensure_edge_label(_VALUE_AT_ELABEL, _STRUCTURED_DATA_VLABEL, _STRUCTURED_DATA_VLABEL)
    
    # Fragment Tree
    ensure_edge_label(_ROOT_ELABEL, _ARTIFACT_VLABEL, _FRAGMENT_VLABEL)
    ensure_edge_label(_CONNECTION_ELABEL, _FRAGMENT_VLABEL, _FRAGMENT_VLABEL)
    
    # Computational
    ensure_edge_label(_CALCULATES_ELABEL, _EXPRESSION_VLABEL, _FRAGMENT_VLABEL)
    ensure_edge_label(_FEEDS_ELABEL, _FRAGMENT_VLABEL, _EXPRESSION_VLABEL)
    ensure_edge_label(_SEQUENCED_IN_ELABEL, _PROPOSITION_VLABEL, _FRAGMENT_VLABEL)
    
    # Pollock Argument Graph
    ensure_edge_label(_SUPPORTS_ELABEL, _FRAGMENT_VLABEL, _PROPOSITION_VLABEL)
    ensure_edge_label(_REBUTS_ELABEL, _FRAGMENT_VLABEL, _PROPOSITION_VLABEL)
    ensure_edge_label(_UNDERCUTS_ELABEL, _FRAGMENT_VLABEL, _PROPOSITION_VLABEL)

    def ensure_index_label(name: str, on: str, base_type: str, by: str) -> None:
        try:
            client.schema_get("indexlabels", name)
            return
        except Exception:
            pass
        try:
            client.schema_create(
                "indexlabels",
                {"name": name, "base_type": base_type, "base_value": on, "index_type": "SECONDARY", "fields": [by]},
            )
        except HugeGraphError as e:
            if e.status in {400, 409}:
                return
            raise

    # Secondary indexes for projection management
    for elabel in [_DERIVATION_ELABEL, _VALUE_AT_ELABEL, _ROOT_ELABEL, _CONNECTION_ELABEL]:
        ensure_index_label(f"{elabel}_by_edge_key", elabel, "EDGE", "edge_key")
        ensure_index_label(f"{elabel}_by_project_id", elabel, "EDGE", "project_id")


@dataclass(frozen=True)
class ProjectionCheckpoint:
    project_id: str
    consumer_name: str
    last_event_id: int


def get_projection_checkpoint(
    cx: psycopg.Connection[Any], *, project_id: str, consumer_name: str
) -> ProjectionCheckpoint | None:
    row = cx.execute(
        """
        SELECT project_id, consumer_name, last_event_id
          FROM graph_edge_projection_checkpoints
         WHERE project_id = %s AND consumer_name = %s
        """,
        (project_id, consumer_name),
    ).fetchone()
    if not row:
        return None
    return ProjectionCheckpoint(
        project_id=str(row["project_id"]),
        consumer_name=str(row["consumer_name"]),
        last_event_id=int(row["last_event_id"]),
    )


def set_projection_checkpoint(
    cx: psycopg.Connection[Any], *, project_id: str, consumer_name: str, last_event_id: int, now_ms: int
) -> None:
    cx.execute(
        """
        INSERT INTO graph_edge_projection_checkpoints (project_id, consumer_name, last_event_id, updated_at)
        VALUES (%s, %s, %s, %s)
        ON CONFLICT (project_id, consumer_name)
        DO UPDATE SET last_event_id = EXCLUDED.last_event_id, updated_at = EXCLUDED.updated_at
        """,
        (project_id, consumer_name, int(last_event_id), int(now_ms)),
    )


def _edge_key_for_event(e: GraphEdgeEvent) -> str:
    return compute_edge_identity_key(
        edge_label=e.edge_label,
        out_id=e.out_id,
        in_id=e.in_id,
        properties=e.properties,
    )


def _subtree_delete_gremlin(*, project_id: str, event: GraphEdgeEvent) -> str:
    handle = str(event.properties["graphDeltaHandle"])
    path = json.dumps(event.properties["graphDeltaPath"], separators=(",", ":"), ensure_ascii=False)
    return (
        f"g.E().has('project_id', '{project_id}')"
        f".has('graphDeltaHandle', '{handle}')"
        f".has('graphDeltaPath', org.apache.tinkerpop.gremlin.process.traversal.TextP.startingWith('{path[:-1]}'))"
        ".drop()"
    )


def _infer_vlabel(id: str, props: dict[str, Any]) -> str:
    if id.startswith("art:"):
        return _ARTIFACT_VLABEL
    
    mime = props.get("mime_type") or props.get("mimeType")
    if mime:
        if "proposition" in mime:
            return _PROPOSITION_VLABEL
        if "expression" in mime:
            return _EXPRESSION_VLABEL
        if "structured-data" in mime:
            return _STRUCTURED_DATA_VLABEL
            
    vtype = props.get("type") or props.get("fragment_type")
    if vtype:
        if vtype.lower() == "proposition":
            return _PROPOSITION_VLABEL
        if vtype.lower() == "expression":
            return _EXPRESSION_VLABEL
        if vtype.lower() == "structured_data":
            return _STRUCTURED_DATA_VLABEL
            
    return _FRAGMENT_VLABEL


def _ensure_vertex(client: HugeGraphClient, *, id: str, project_id: str, properties: dict[str, Any]) -> str:
    vlabel = _infer_vlabel(id, properties)
    id_strategy = _id_strategy(client, vlabel)
    payload_id = id if id_strategy == "CUSTOMIZE_STRING" else None

    props = {"id": id, "project_id": project_id}
    for key in ["type", "mime_type", "salience", "confidence"]:
        val = properties.get(key)
        if val is not None:
            props[key] = val

    created = client.create_vertex(
        label=vlabel,
        vertex_id=payload_id,
        properties=props,
    )
    v_id = created.get("id") if isinstance(created, dict) else None
    if v_id:
        return str(v_id)

    if id_strategy == "PRIMARY_KEY":
        vlabel_id = _vlabel_id(client, vlabel)
        if vlabel_id is not None:
            return f"{vlabel_id}:{id}"

    return id


def _infer_elabel(edge_label: str) -> str:
    if edge_label == "graph:value_at":
        return _VALUE_AT_ELABEL
    if edge_label.startswith("derivation:"):
        return _DERIVATION_ELABEL
    if edge_label == "root":
        return _ROOT_ELABEL
    if edge_label == "connection":
        return _CONNECTION_ELABEL
    if edge_label == "calculates":
        return _CALCULATES_ELABEL
    if edge_label == "feeds":
        return _FEEDS_ELABEL
    if edge_label == "sequenced-in":
        return _SEQUENCED_IN_ELABEL
    if edge_label == "supports":
        return _SUPPORTS_ELABEL
    if edge_label == "rebuts":
        return _REBUTS_ELABEL
    if edge_label == "undercuts":
        return _UNDERCUTS_ELABEL
    return _DERIVATION_ELABEL


def replay_graph_edge_events_until_done(
    cx: psycopg.Connection[Any],
    *,
    client: HugeGraphClient,
    project_id: str,
    consumer_name: str = "hugegraph-projection",
    edge_label_prefix: str = "derivation:",
    batch_size: int = 500,
) -> int:
    ensure_ikam_projection_schema(client)

    checkpoint = get_projection_checkpoint(cx, project_id=project_id, consumer_name=consumer_name)
    after_id = checkpoint.last_event_id if checkpoint else 0
    last_processed = after_id

    while True:
        events = list_graph_edge_events(
            cx,
            project_id=project_id,
            after_id=after_id,
            limit=int(batch_size),
        )
        if not events:
            break

        for e in events:
            after_id = e.id
            edge_key = _edge_key_for_event(e)
            if not edge_key:
                continue

            if e.op == "delete":
                if is_subtree_graph_delta_delete(e):
                    gremlin = _subtree_delete_gremlin(project_id=project_id, event=e)
                else:
                    # Best effort delete by Gremlin for now as it handles complex properties well
                    gremlin = f"g.E().has('project_id', '{project_id}').has('edge_key', '{edge_key}').drop()"
                client.gremlin_query(gremlin)
            else:
                out_v_props = dict(e.properties.get("out_properties") or {})
                in_v_props = dict(e.properties.get("in_properties") or {})
                
                out_vertex_id = _ensure_vertex(client, id=e.out_id, project_id=project_id, properties=out_v_props)
                in_vertex_id = _ensure_vertex(client, id=e.in_id, project_id=project_id, properties=in_v_props)

                elabel = _infer_elabel(e.edge_label)
                
                props = {
                    "edge_key": edge_key,
                    "project_id": project_id,
                    "edge_label": e.edge_label,
                    "t": int(e.t),
                }
                for key in ["derivation_id", "derivation_type", "confidence"]:
                    val = e.properties.get(key)
                    if val is not None:
                        props[key] = val
                if e.properties.get("graphDeltaHandle") is not None:
                    props["graphDeltaHandle"] = e.properties["graphDeltaHandle"]
                if e.properties.get("graphDeltaPath") is not None:
                    props["graphDeltaPath"] = json.dumps(
                        e.properties["graphDeltaPath"], separators=(",", ":"), ensure_ascii=False
                    )
                for key in ["envType", "envId"]:
                    val = e.properties.get(key)
                    if val is not None:
                        props[key] = val

                try:
                    client.create_edge(
                        label=elabel,
                        out_v=out_vertex_id,
                        in_v=in_vertex_id,
                        out_v_label=_infer_vlabel(e.out_id, out_v_props),
                        in_v_label=_infer_vlabel(e.in_id, in_v_props),
                        properties=props,
                    )
                except HugeGraphError as err:
                    if err.status in {409}:
                        pass
                    else:
                        raise

            last_processed = max(last_processed, e.id)

        set_projection_checkpoint(
            cx,
            project_id=project_id,
            consumer_name=consumer_name,
            last_event_id=last_processed,
            now_ms=max(int(events[-1].t), 0),
        )
        cx.commit()

    return last_processed


async def replay_graph_edge_events_until_done_async(
    cx: psycopg.Connection[Any],
    *,
    client: HugeGraphClient,
    project_id: str,
    consumer_name: str = "hugegraph-projection",
    edge_label_prefix: str = "derivation:",
    batch_size: int = 500,
) -> int:
    """Async wrapper for projection replay worker integration.

    Keeps the core projector synchronous/deterministic while allowing async worker
    loops to schedule projection work without blocking the event loop.
    """

    return await asyncio.to_thread(
        replay_graph_edge_events_until_done,
        cx,
        client=client,
        project_id=project_id,
        consumer_name=consumer_name,
        edge_label_prefix=edge_label_prefix,
        batch_size=batch_size,
    )
