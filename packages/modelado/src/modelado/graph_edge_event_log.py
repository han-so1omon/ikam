from __future__ import annotations

import hashlib
import json
import time
from dataclasses import dataclass
from typing import Any, Mapping, Optional

from psycopg import Connection
from psycopg.types.json import Json

from modelado.core.execution_context import ExecutionPolicyViolation, get_execution_context, require_write_scope


@dataclass(frozen=True)
class GraphEdgeEvent:
    id: int
    project_id: str
    op: str
    edge_label: str
    out_id: str
    in_id: str
    properties: dict[str, Any]
    t: int
    idempotency_key: Optional[str]


@dataclass(frozen=True)
class GraphEdgeEventCorrelationError(ValueError):
    missing_keys: tuple[str, ...]

    def __str__(self) -> str:  # pragma: no cover
        missing = ",".join(self.missing_keys)
        return f"Missing correlation fields for graph edge event: {missing}"


def _require_non_empty_str(field: str, value: object) -> None:
    if value is None:
        raise GraphEdgeEventCorrelationError(missing_keys=(field,))
    if not isinstance(value, str):
        raise GraphEdgeEventCorrelationError(missing_keys=(f"{field} (must be str)",))
    if not value.strip():
        raise GraphEdgeEventCorrelationError(missing_keys=(field,))


def _canonical_json(value: Mapping[str, Any]) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def compute_edge_event_idempotency_key(
    *,
    op: str,
    edge_label: str,
    out_id: str,
    in_id: str,
    properties: Mapping[str, Any],
) -> str:
    """Compute a deterministic idempotency key for retries.

    This must be stable across processes for the same logical event.
    """

    canonical = "|".join(
        [
            op,
            edge_label,
            out_id,
            in_id,
            _canonical_json(dict(properties)),
        ]
    )
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def compute_edge_identity_key(
    *,
    edge_label: str,
    out_id: str,
    in_id: str,
    properties: Mapping[str, Any],
) -> str:
    """Compute a deterministic identity key for the effective edge.

    Unlike `compute_edge_event_idempotency_key`, this key is independent of the
    event op so that `upsert` and `delete` events target the same effective edge
    during folding and replay.

    The key intentionally ignores mutable metadata (like confidence scores),
    while retaining stable discriminators for known edge families.
    """

    props = dict(properties)

    # All edges use knowledge: prefix. Extract stable discriminators.
    # Include environment/pipeline qualifiers to avoid cross-environment
    # collisions for the same relation fragment.
    if "relationFragmentId" in props or "derivationId" in props:
        stable_keys = [
            "relationFragmentId",
            "derivationId",
            "envType",
            "envId",
            "pipelineId",
            "pipelineRunId",
            "operationId",
        ]
        scoped: dict[str, Any] = {}
        for key in stable_keys:
            if key in props:
                scoped[key] = props.get(key)
        props = scoped

    return compute_edge_event_idempotency_key(
        op="upsert",
        edge_label=edge_label,
        out_id=out_id,
        in_id=in_id,
        properties=props,
    )


def compute_relation_commit_receipt_id(
    *,
    project_id: str,
    overlay_id: str,
    committed_fragment_ids: list[str] | tuple[str, ...],
    edge_idempotency_keys: list[str] | tuple[str, ...],
) -> str:
    """Compute deterministic commit receipt identifier for relation commits."""

    canonical = "|".join(
        [
            project_id,
            overlay_id,
            ",".join(sorted(str(item) for item in committed_fragment_ids)),
            ",".join(sorted(str(item) for item in edge_idempotency_keys)),
        ]
    )
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def append_graph_edge_event(
    cx: Connection,
    *,
    project_id: str,
    op: str,
    edge_label: str,
    out_id: str,
    in_id: str,
    properties: Optional[Mapping[str, Any]] = None,
    t: Optional[int] = None,
    idempotency_key: Optional[str] = None,
) -> Optional[GraphEdgeEvent]:
    """Append an edge mutation to the authoritative Postgres event log.

    The log is append-only. Retry safety is provided by a unique
    (project_id, idempotency_key) index when idempotency_key is set.

    Returns the inserted event, or None if it was a no-op due to an
    idempotency conflict.
    """

    # Correlation contract (canonical): project_id + idempotency_key.
    # We compute idempotency_key if omitted, but if provided it must be non-empty.
    _require_non_empty_str("project_id", project_id)
    if idempotency_key is not None:
        _require_non_empty_str("idempotency_key", idempotency_key)

    # If an upstream signed write scope is present, ensure this edge event matches it.
    # This is a guard-only enforcement: unsigned/background flows without a scope
    # continue to work as before.
    ctx = get_execution_context()
    if ctx is not None and ctx.write_scope is not None:
        scope = require_write_scope("append_graph_edge_event")
        if scope.project_id != project_id:
            raise ExecutionPolicyViolation(
                "Execution policy violation: graph edge event project_id does not match active write scope "
                f"(event_project_id={project_id} scope_project_id={scope.project_id})"
            )

    properties_dict = dict(properties or {})
    t_ms = int(t if t is not None else time.time() * 1000)

    key = idempotency_key
    if key is None:
        key = compute_edge_event_idempotency_key(
            op=op,
            edge_label=edge_label,
            out_id=out_id,
            in_id=in_id,
            properties=properties_dict,
        )

    _require_non_empty_str("idempotency_key", key)

    row = cx.execute(
        """
        INSERT INTO graph_edge_events (
          project_id, op, edge_label, out_id, in_id, properties, t, idempotency_key
        )
        VALUES (%s, %s, %s, %s, %s, %s, %s, %s)
                ON CONFLICT (project_id, idempotency_key) WHERE idempotency_key IS NOT NULL DO NOTHING
        RETURNING id, project_id, op, edge_label, out_id, in_id, properties, t, idempotency_key
        """,
        (
            project_id,
            op,
            edge_label,
            out_id,
            in_id,
            Json(properties_dict),
            t_ms,
            key,
        ),
    ).fetchone()

    if not row:
        return None

    def _get(col: str, idx: int) -> Any:
        try:
            return row[col]
        except Exception:
            return row[idx]

    return GraphEdgeEvent(
        id=int(_get("id", 0)),
        project_id=str(_get("project_id", 1)),
        op=str(_get("op", 2)),
        edge_label=str(_get("edge_label", 3)),
        out_id=str(_get("out_id", 4)),
        in_id=str(_get("in_id", 5)),
        properties=dict((_get("properties", 6) or {})),
        t=int(_get("t", 7)),
        idempotency_key=_get("idempotency_key", 8),
    )


def list_graph_edge_events(
    cx: Connection,
    *,
    project_id: str,
    after_id: int = 0,
    limit: int = 500,
    op: Optional[str] = None,
    edge_label: Optional[str] = None,
    out_id: Optional[str] = None,
    in_id: Optional[str] = None,
) -> list[GraphEdgeEvent]:
    """List edge events for a project in deterministic replay order."""

    conditions: list[str] = ["project_id = %s", "id > %s"]
    params: list[Any] = [project_id, int(after_id)]

    if op:
        conditions.append("op = %s")
        params.append(op)
    if edge_label:
        conditions.append("edge_label = %s")
        params.append(edge_label)
    if out_id:
        conditions.append("out_id = %s")
        params.append(out_id)
    if in_id:
        conditions.append("in_id = %s")
        params.append(in_id)

    where_clause = " AND ".join(conditions)

    rows = cx.execute(
        f"""
        SELECT id, project_id, op, edge_label, out_id, in_id, properties, t, idempotency_key
          FROM graph_edge_events
         WHERE {where_clause}
         ORDER BY id ASC
         LIMIT %s
        """,
        (*params, int(limit)),
    ).fetchall()

    out: list[GraphEdgeEvent] = []
    for row in rows:
        def _get(col: str, idx: int) -> Any:
            try:
                return row[col]
            except Exception:
                return row[idx]

        out.append(
            GraphEdgeEvent(
                id=int(_get("id", 0)),
                project_id=str(_get("project_id", 1)),
                op=str(_get("op", 2)),
                edge_label=str(_get("edge_label", 3)),
                out_id=str(_get("out_id", 4)),
                in_id=str(_get("in_id", 5)),
                properties=dict((_get("properties", 6) or {})),
                t=int(_get("t", 7)),
                idempotency_key=_get("idempotency_key", 8),
            )
        )
    return out
