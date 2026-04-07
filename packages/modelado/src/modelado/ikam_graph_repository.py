"""Repository for persisting IKAM graph entities (fragments, artifacts, derivations)."""

from __future__ import annotations

from typing import Any, Optional, cast

import uuid
import datetime as _dt
import hashlib
import json as _json

import psycopg
from psycopg.abc import Query
from ikam.graph import StoredFragment
from ikam.fragments import Fragment as DomainFragment
import ikam.adapters as ikam_adapters
from ikam.ir import OpShape, OpInstance

from modelado.graph_edge_event_log import append_graph_edge_event
from modelado.environment_scope import (
    EnvironmentScope,
    add_scope_qualifiers,
    parse_reference_scopes,
    validate_cross_environment_mutation,
)
from modelado.core.execution_context import (
    ExecutionPolicyViolation,
    get_execution_context,
    require_write_scope,
)

# Observability
try:
    from modelado import ikam_metrics
except ImportError:
    ikam_metrics = None  # Metrics disabled if prometheus_client not installed

def _require_ikam_write(operation: str) -> None:
    """Enforce execution policy for IKAM write operations.

    Policy:
    - All IKAM writes must run under an active ExecutionContext.
    - If the caller is the system actor (ExecutionContext.actor_id is None), a
      verified WriteScope must be present (strict-fail for unsigned system writes).
    """

    ctx = get_execution_context()
    if ctx is None:
        raise ExecutionPolicyViolation(
            f"Execution policy violation: '{operation}' requires an execution context (ctx=None)"
        )

    # Only system actor writes are signature-gated.
    if ctx.actor_id is None:
        require_write_scope(operation)


def _resolve_project_id(cx: psycopg.Connection[Any]) -> str:
    """Resolve a project_id for IKAM writes.

    Priority:
    1) ExecutionContext write scope project_id
    2) Test connection attribute (_test_project_id)
    3) Default project (bootstrap seed)
    """

    ctx = get_execution_context()
    if ctx and ctx.write_scope and ctx.write_scope.project_id:
        return str(ctx.write_scope.project_id)

    test_project_id = getattr(cx, "_test_project_id", None)
    if isinstance(test_project_id, str) and test_project_id:
        return test_project_id

    return "default-project"


def insert_fragment(cx: psycopg.Connection[Any], fragment: StoredFragment) -> None:
    """Insert a CAS fragment (idempotent on conflict).
    
    Note: This function accepts storage-layer graph.Fragment.
    For domain fragments, use insert_domain_fragment() instead.
    """
    _require_ikam_write("insert_fragment")
    with cx.cursor() as cur:
        # Check if fragment already exists (CAS hit)
        cur.execute("SELECT 1 FROM ikam_fragments WHERE id = %s", (fragment.id,))
        exists = cur.fetchone() is not None
        
        cur.execute(
            """
            INSERT INTO ikam_fragments (id, mime_type, size, bytes)
            VALUES (%s, %s, %s, %s)
            ON CONFLICT (id) DO NOTHING
            """,
            (fragment.id, fragment.mime_type, fragment.size, fragment.bytes),
        )
        
        # Track CAS hit/miss
        if ikam_metrics:
            if exists:
                ikam_metrics.record_cas_hit()
            else:
                ikam_metrics.record_cas_miss()


def store_fragment(
    cx: psycopg.Connection[Any],
    fragment: DomainFragment,
    *,
    project_id: str,
    ref: str = "refs/heads/main",
    operation_id: str | None = None,
) -> None:
    """Store a domain Fragment in ikam_fragment_store (idempotent via CAS).

    Args:
        cx: psycopg connection
        fragment: Domain Fragment (from ikam.fragments or ikam.forja.cas)
        project_id: Project scope
        ref: Canonical branch/ref scope for membership
        operation_id: Optional operation that produced this fragment
    """
    _require_ikam_write("store_fragment")
    if fragment.cas_id is None:
        raise ValueError("store_fragment requires a fragment with cas_id set")
    scope_ref = EnvironmentScope(ref=ref).ref
    with cx.cursor() as cur:
        cur.execute(
            """
            INSERT INTO ikam_fragment_store (cas_id, ref, operation_id, project_id, value, mime_type)
            VALUES (%s, %s, %s, %s, %s::jsonb, %s)
            ON CONFLICT (cas_id, ref, COALESCE(operation_id, '')) DO NOTHING
            """,
            (
                fragment.cas_id,
                scope_ref,
                operation_id,
                project_id,
                _json.dumps(fragment.value),
                fragment.mime_type,
            ),
        )






def integrate_parse_outputs(
    cx: psycopg.Connection[Any] | None,
    *,
    artifact_id: str,
    derived_edges: list[tuple[str, str]],
) -> dict[str, Any]:
    if cx is None:
        return {"artifact_id": artifact_id, "new_fragment_id": derived_edges[0][1] if derived_edges else None}
    _require_ikam_write("integrate_parse_outputs")
    project_id = _resolve_project_id(cx)
    new_fragment_id = derived_edges[0][1] if derived_edges else None
    derivation_id = str(uuid.uuid4())
    for source_id, target_id in derived_edges:
        append_graph_edge_event(
            cx,
            project_id=str(project_id),
            op="upsert",
            edge_label="knowledge:parse",
            out_id=source_id,
            in_id=target_id,
            properties={
                "artifact_id": artifact_id,
                "relation": "parse_output",
                "derivationId": derivation_id,
                "derivationType": "parse",
            },
        )
    return {"artifact_id": artifact_id, "new_fragment_id": new_fragment_id}


def emit_edge_event(
    cx: psycopg.Connection[Any],
    *,
    source_id: str,
    target_id: str,
    predicate: str,
    project_id: str,
    properties: dict[str, Any] | None = None,
    ref: str,
    pipeline_id: str | None = None,
    pipeline_run_id: str | None = None,
    operation_id: str | None = None,
    reference_scopes: list[dict[str, Any]] | None = None,
    delta_intents: list[dict[str, Any]] | None = None,
) -> None:
    """Emit a knowledge-graph edge event using ``knowledge:<predicate>`` format.

    Delegates to the append-only ``graph_edge_events`` table via
    ``append_graph_edge_event``. All edges use the ``knowledge:`` prefix
    exclusively — the legacy ``derivation:`` prefix is removed.
    """
    _require_ikam_write("emit_edge_event")
    target_scope = EnvironmentScope(ref=ref)
    parsed_refs = parse_reference_scopes(reference_scopes)
    validate_cross_environment_mutation(
        target_scope=target_scope,
        reference_scopes=parsed_refs,
        delta_intents=delta_intents,
    )
    edge_label = f"knowledge:{predicate}"
    props = dict(properties) if properties else {}
    props = add_scope_qualifiers(
        properties=props,
        scope=target_scope,
        pipeline_id=pipeline_id,
        pipeline_run_id=pipeline_run_id,
        operation_id=operation_id,
    )
    append_graph_edge_event(
        cx,
        project_id=project_id,
        op="upsert",
        edge_label=edge_label,
        out_id=source_id,
        in_id=target_id,
        properties=props,
    )


def promote_fragment(
    cx: psycopg.Connection[Any],
    fragment_ids: str | list[str],
    *,
    source_ref: str,
    target_ref: str,
) -> None:
    """Promote selected fragments from one ref to another.

    ``source_ref`` and ``target_ref`` are the canonical promotion API.
    """
    _require_ikam_write("promote_fragment")
    normalized_source_ref = EnvironmentScope(ref=source_ref).ref
    normalized_target_ref = EnvironmentScope(ref=target_ref).ref
    normalized_fragment_ids = [str(fragment_ids)] if isinstance(fragment_ids, str) else [str(item) for item in fragment_ids]
    if not normalized_fragment_ids:
        raise ValueError("fragment_ids must be non-empty")
    if normalized_source_ref == normalized_target_ref:
        raise ValueError("source_ref and target_ref must differ")

    with cx.cursor() as cur:
        cur.execute(
            "UPDATE ikam_fragment_store SET ref = %s WHERE cas_id = ANY(%s) AND ref = %s",
            (normalized_target_ref, normalized_fragment_ids, normalized_source_ref),
        )


def insert_domain_fragment(
    cx: psycopg.Connection[Any],
    domain_fragment: Any,
    *,
    domain_id_to_cas_id: dict[str, str] | None = None,
) -> None:
    """Insert a domain fragment into CAS storage only.

    Immutable fragment objects store hierarchy/content; we persist only the CAS
    fragment bytes here.
    """
    _require_ikam_write("insert_domain_fragment")
    storage_fragment = ikam_adapters.v3_to_storage(domain_fragment)
    domain_fragment_id = getattr(domain_fragment, "id", None) or getattr(domain_fragment, "fragment_id", None)
    if domain_id_to_cas_id is not None and domain_fragment_id is not None:
        storage_fragment = storage_fragment.model_copy(
            update={"id": domain_id_to_cas_id.get(str(domain_fragment_id), storage_fragment.id)}
        )
    insert_fragment(cx, storage_fragment)


def insert_domain_fragments(
    cx: psycopg.Connection[Any],
    fragments: list[Any],
) -> dict[str, str]:
    """Insert a set of domain fragments while preserving parent/child hierarchy.

    Decomposition often yields temporary domain ids for fragments. The database schema
    stores hierarchy edges via CAS ids, so we compute a domain→CAS map up-front and
    use it to resolve parent_fragment_id consistently.

    Returns:
        Mapping of domain fragment id → CAS id.
    """
    _require_ikam_write("insert_domain_fragments")
    if not fragments:
        return {}

    domain_id_to_cas_id = {
        str(fragment_id): ikam_adapters.v3_to_storage(frag).id
        for frag in fragments
        for fragment_id in [getattr(frag, "id", None) or getattr(frag, "fragment_id", None)]
        if fragment_id is not None
    }

    # Insert parents before children. For IKAM decompositions, parent fragments should
    # always be at a lower level.
    for frag in sorted(fragments, key=lambda f: getattr(f, "level", 0)):
        insert_domain_fragment(cx, frag, domain_id_to_cas_id=domain_id_to_cas_id)

    return domain_id_to_cas_id


def get_fragment_by_id(cx: psycopg.Connection[Any], fragment_id: str) -> StoredFragment | None:
    """Retrieve a storage fragment by CAS ID (minimal CAS model)."""
    with cx.cursor() as cur:
        cur.execute("SELECT id, mime_type, size, bytes FROM ikam_fragments WHERE id = %s", (fragment_id,))
        row = cur.fetchone()
        if not row:
            return None
        return StoredFragment(id=row["id"], mime_type=row["mime_type"], size=row["size"], bytes=bytes(row["bytes"]))


def _get_manifest_for_artifact(cx: psycopg.Connection[Any], artifact_id: str) -> dict[str, Any] | None:
    with cx.cursor() as cur:
        row = cur.execute(
            """
            SELECT o.manifest
              FROM ikam_artifacts a
              JOIN ikam_fragment_objects o
                ON o.object_id = a.head_object_id
             WHERE a.id = %s
            """,
            (uuid.UUID(artifact_id),),
        ).fetchone()
    if not row:
        return None
    return row["manifest"] if isinstance(row, dict) else row[0]





def get_domain_fragments_for_artifact(cx: psycopg.Connection[Any], artifact_id: str) -> list[dict[str, Any]]:
    """Retrieve all domain fragments for an artifact with hierarchy, content, and radicals.
    
    Returns fragments ordered by level (L0 → L3) for hierarchical display.
    
    Args:
        cx: psycopg connection
        artifact_id: UUID of the artifact
        
    Returns:
        List of domain fragments with full metadata, content, and radicals
    """
    manifest = _get_manifest_for_artifact(cx, artifact_id)
    if not manifest:
        return []
    entries = ikam_adapters.normalize_fragment_object_entries(manifest)  # type: ignore[attr-defined]
    fragment_ids = [entry["fragmentId"] for entry in entries if entry.get("fragmentId")]
    if not fragment_ids:
        return []

    with cx.cursor() as cur:
        cur.execute(
            "SELECT id, mime_type, size, bytes FROM ikam_fragments WHERE id = ANY(%s)",
            (fragment_ids,),
        )
        rows = cur.fetchall() or []

    rows_by_id: dict[str, Any] = {}
    for row in rows:
        frag_id = row["id"] if isinstance(row, dict) else row[0]
        rows_by_id[str(frag_id)] = row

    domain_fragments: list[dict[str, Any]] = []
    for entry in entries:
        frag_id = entry.get("fragmentId")
        if not frag_id:
            continue
        row = rows_by_id.get(str(frag_id))
        if not row:
            continue
        mime_type = row["mime_type"] if isinstance(row, dict) else row[1]
        size = row["size"] if isinstance(row, dict) else row[2]
        payload = row["bytes"] if isinstance(row, dict) else row[3]
        parent_id = entry.get("parentFragmentId")
        try:
            fragment = ikam_adapters.v3_fragment_from_cas_bytes(cas_id=str(frag_id), payload=bytes(payload))
            domain_fragments.append(
                {
                    "id": str(frag_id),
                    "artifact_id": str(artifact_id),
                    "parentFragmentId": parent_id,
                    "mime_type": mime_type or fragment.mime_type,
                    "size": size,
                    "bytes": bytes(payload),
                    "cas_id": fragment.cas_id,
                    "value": fragment.value,
                }
            )
        except Exception:
            continue
    return domain_fragments


def list_domain_fragment_headers_for_artifact(cx: psycopg.Connection[Any], artifact_id: str) -> list[dict[str, Any]]:
    """List domain fragment headers for an artifact without hydrating content.

    This exists to support UI paths that only need IDs + hierarchy + salience (and
    optionally radicals) without paying the cost of loading/serializing large
    fragment content payloads (e.g., sheet cell maps).

    Uses IKAM tables exclusively.
    """
    manifest = _get_manifest_for_artifact(cx, artifact_id)
    if not manifest:
        return []
    entries = ikam_adapters.normalize_fragment_object_entries(manifest)  # type: ignore[attr-defined]
    fragment_ids = [entry["fragmentId"] for entry in entries if entry.get("fragmentId")]
    if not fragment_ids:
        return []

    with cx.cursor() as cur:
        cur.execute(
            "SELECT id, mime_type, size FROM ikam_fragments WHERE id = ANY(%s)",
            (fragment_ids,),
        )
        rows = cur.fetchall() or []

    rows_by_id: dict[str, Any] = {}
    for row in rows:
        frag_id = row["id"] if isinstance(row, dict) else row[0]
        rows_by_id[str(frag_id)] = row

    out: list[dict[str, Any]] = []
    for entry in entries:
        fragment_id = entry.get("fragmentId")
        if not fragment_id:
            continue
        row = rows_by_id.get(str(fragment_id))
        mime_type = row["mime_type"] if isinstance(row, dict) else (row[1] if row else None)
        size = row["size"] if isinstance(row, dict) else (row[2] if row else None)
        out.append(
            {
                "id": fragment_id,
                "artifact_id": str(artifact_id),
                "level": entry.get("level"),
                "type": entry.get("type"),
                "parent_fragment_id": entry.get("parentFragmentId"),
                "salience": entry.get("salience"),
                "created_at": None,
                "updated_at": None,
                "mime_type": mime_type,
                "size": size,
                "radical_refs": entry.get("radicalRefs", []),
            }
        )
    return out








def get_provenance_events_for_artifact(cx: psycopg.Connection[Any], artifact_id: str) -> list[dict[str, Any]]:
    """Retrieve provenance events for an artifact ordered by creation time."""
    with cx.cursor() as cur:
        cur.execute(
            "SELECT id, event_type, author_id, derivation_id, created_at, details FROM ikam_provenance_events WHERE artifact_id = %s ORDER BY created_at ASC",
            (uuid.UUID(artifact_id),),
        )
        events = []
        for row in cur.fetchall():
            events.append(
                {
                    "id": str(row["id"]),
                    "event_type": row["event_type"],
                    "author_id": str(row["author_id"]) if row["author_id"] else None,
                    "derivation_id": str(row["derivation_id"]) if row["derivation_id"] else None,
                    "created_at": row["created_at"],
                    "details": row["details"],
                }
            )
        return events


def validate_provenance_event_details(event_type: str, details: Optional[dict[str, Any]]) -> None:
    """Validate required details fields for select provenance event types.

    This keeps the provenance log semantically complete for downstream
    reproducibility and auditability checks.

    Raises:
        ValueError: when required keys are missing.
    """

    if event_type == "Rendered":
        required_keys = {"seed", "rendererVersion", "variationId"}
        if not details or not required_keys.issubset(details.keys()):
            missing = required_keys - (set(details.keys()) if details else set())
            raise ValueError(f"Rendered event requires details keys: {', '.join(sorted(missing))}")

    if event_type == "SystemMutated":
        # Keep this aligned with SignedMutationIntent (base-api) for verifiable audit trails.
        required_keys = {
            "operation",
            "project_id",
            "timestamp",
            "nonce",
            "payload_hash",
            "agent_id",
            "key_fingerprint",
            "signature",
        }
        if not details or not required_keys.issubset(details.keys()):
            missing = required_keys - (set(details.keys()) if details else set())
            raise ValueError(f"SystemMutated event requires details keys: {', '.join(sorted(missing))}")


def build_system_mutation_details(
    *,
    operation: str,
    project_id: str,
    payload: Any | None = None,
    agent_id: str,
    key_fingerprint: str,
    signature: str,
    timestamp: str | None = None,
    nonce: str | None = None,
    payload_hash: str | None = None,
    extra: Optional[dict[str, Any]] = None,
) -> dict[str, Any]:
    """Build a SystemMutated event details payload.

    This is intentionally small and schema-light: downstream validation enforces
    required keys but does not require a specific hashing algorithm.
    """

    ts = timestamp or _dt.datetime.now(_dt.timezone.utc).isoformat()
    n = nonce or str(uuid.uuid4())

    computed_hash = payload_hash
    if computed_hash is None:
        if payload is None:
            computed_hash = "sha256:"
        elif isinstance(payload, (bytes, bytearray)):
            computed_hash = "sha256:" + hashlib.sha256(bytes(payload)).hexdigest()
        else:
            try:
                raw = _json.dumps(payload, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
            except Exception:
                raw = repr(payload).encode("utf-8")
            computed_hash = "sha256:" + hashlib.sha256(raw).hexdigest()

    details: dict[str, Any] = {
        "operation": operation,
        "project_id": project_id,
        "timestamp": ts,
        "nonce": n,
        "payload_hash": computed_hash,
        "agent_id": agent_id,
        "key_fingerprint": key_fingerprint,
        "signature": signature,
    }
    if extra:
        details.update(extra)
    return details


def record_system_mutation_event(
    cx: psycopg.Connection[Any],
    *,
    artifact_id: str,
    operation: str,
    project_id: str,
    payload: Any | None = None,
    agent_id: str,
    key_fingerprint: str,
    signature: str,
    author_id: Optional[str] = None,
    extra: Optional[dict[str, Any]] = None,
) -> str:
    """Record a canonical audit event for a system mutation.

    Note: This does not implement signature verification; it records the provided
    details for later auditing and reproducibility checks.
    """

    # If a signed write scope is present upstream, enforce consistency and record
    # the signed envelope fields (nonce/payload_hash) rather than recomputing.
    ctx = get_execution_context()
    nonce: str | None = None
    payload_hash: str | None = None
    if ctx is not None and ctx.write_scope is not None:
        scope = require_write_scope("record_system_mutation_event")
        if scope.project_id != project_id:
            raise ExecutionPolicyViolation(
                "Execution policy violation: SystemMutated project_id does not match active write scope "
                f"(event_project_id={project_id} scope_project_id={scope.project_id})"
            )
        if scope.operation != operation:
            raise ExecutionPolicyViolation(
                "Execution policy violation: SystemMutated operation does not match active write scope "
                f"(event_operation={operation} scope_operation={scope.operation})"
            )
        if scope.agent_id and agent_id != scope.agent_id:
            raise ExecutionPolicyViolation(
                "Execution policy violation: SystemMutated agent_id does not match active write scope "
                f"(event_agent_id={agent_id} scope_agent_id={scope.agent_id})"
            )
        if scope.key_fingerprint and key_fingerprint != scope.key_fingerprint:
            raise ExecutionPolicyViolation(
                "Execution policy violation: SystemMutated key_fingerprint does not match active write scope "
                f"(event_key_fingerprint={key_fingerprint} scope_key_fingerprint={scope.key_fingerprint})"
            )
        if scope.signature and signature != scope.signature:
            raise ExecutionPolicyViolation(
                "Execution policy violation: SystemMutated signature does not match active write scope"
            )

        # Prefer scope fields for recording to guarantee the audit record matches
        # the verified envelope exactly.
        agent_id = scope.agent_id or agent_id
        key_fingerprint = scope.key_fingerprint or key_fingerprint
        signature = scope.signature or signature
        nonce = scope.nonce
        payload_hash = scope.payload_hash

    if agent_id is None or key_fingerprint is None or signature is None:
        raise ValueError(
            "record_system_mutation_event requires agent_id, key_fingerprint, and signature"
        )

    details = build_system_mutation_details(
        operation=operation,
        project_id=project_id,
        payload=payload,
        agent_id=agent_id,
        key_fingerprint=key_fingerprint,
        signature=signature,
        nonce=nonce,
        payload_hash=payload_hash,
        extra=extra,
    )
    return record_provenance_event(
        cx,
        artifact_id=artifact_id,
        event_type="SystemMutated",
        author_id=author_id,
        derivation_id=None,
        details=details,
    )


def record_signed_system_mutation_event(
    cx: psycopg.Connection[Any],
    *,
    artifact_id: str,
    payload: Any | None = None,
    author_id: Optional[str] = None,
    extra: Optional[dict[str, Any]] = None,
) -> str:
    """Record a SystemMutated event using the active write scope.

    This is the repo-layer chokepoint for signed automation mutations.
    The required fields are sourced from ExecutionContext.write_scope.
    """

    scope = require_write_scope("record_signed_system_mutation_event")

    missing: list[str] = []
    if not scope.agent_id:
        missing.append("agent_id")
    if not scope.key_fingerprint:
        missing.append("key_fingerprint")
    if not scope.signature:
        missing.append("signature")
    if not scope.nonce:
        missing.append("nonce")
    if not scope.payload_hash:
        missing.append("payload_hash")
    if missing:
        raise ValueError(
            "write_scope is missing required fields for signed SystemMutated audit: "
            + ", ".join(missing)
        )

    assert scope.agent_id is not None
    assert scope.key_fingerprint is not None
    assert scope.signature is not None

    details = build_system_mutation_details(
        operation=scope.operation,
        project_id=scope.project_id,
        payload=payload,
        agent_id=scope.agent_id,
        key_fingerprint=scope.key_fingerprint,
        signature=scope.signature,
        nonce=scope.nonce,
        payload_hash=scope.payload_hash,
        extra=extra,
    )
    return record_provenance_event(
        cx,
        artifact_id=artifact_id,
        event_type="SystemMutated",
        author_id=author_id,
        derivation_id=None,
        details=details,
    )


def record_provenance_event(
    cx: psycopg.Connection[Any],
    *,
    artifact_id: str,
    event_type: str,
    author_id: Optional[str] = None,
    derivation_id: Optional[str] = None,
    details: Optional[dict[str, Any]] = None,
    fragment_id: Optional[str] = None,
    operation_id: Optional[str] = None,
) -> str:
    """Record a provenance event.

    Args:
        cx: psycopg connection (transaction handled by caller)
        artifact_id: UUID of artifact
        event_type: Created|Modified|Derived|Rendered|SystemMutated
        author_id: optional UUID of actor
        derivation_id: optional UUID of related derivation
        details: optional structured metadata (seed, rendererVersion, variationId, etc.)
        fragment_id: optional CAS ID of the fragment this event relates to
        operation_id: optional operation identifier for pipeline traceability

    Returns:
        The UUID (string) of the created event.
        
    Raises:
        ValueError: If event_type requires details keys and they are missing.
    """
    import json

    _require_ikam_write("record_provenance_event")

    validate_provenance_event_details(event_type, details)
    
    ev_id = str(uuid.uuid4())
    with cx.cursor() as cur:
        cur.execute(
            """
            INSERT INTO ikam_provenance_events
                (id, artifact_id, derivation_id, event_type, author_id, details, fragment_id, operation_id)
            VALUES (%s, %s, %s, %s, %s, %s, %s, %s)
            ON CONFLICT (id) DO NOTHING
            """,
            (
                uuid.UUID(ev_id),
                uuid.UUID(artifact_id),
                uuid.UUID(derivation_id) if derivation_id else None,
                event_type,
                uuid.UUID(author_id) if author_id else None,
                json.dumps(details) if details else None,
                fragment_id,
                operation_id,
            ),
        )
    
    # Track provenance event metrics
    if ikam_metrics:
        ikam_metrics.record_provenance_event(event_type)
    
    return ev_id


def emit_created_event(
    cx: psycopg.Connection[Any],
    artifact_id: str,
    author_id: Optional[str] = None,
    details: Optional[dict[str, Any]] = None,
) -> str:
    """Emit a Created provenance event for new artifacts.
    
    Convenience wrapper around record_provenance_event with event_type='Created'.
    
    Args:
        cx: Database connection
        artifact_id: UUID of newly created artifact
        author_id: Optional author UUID
        details: Optional metadata (e.g., source, import_method, initial_content_hash)
        
    Returns:
        Event UUID
        
    Example:
        >>> emit_created_event(cx, artifact_id, author_id, {"source": "user_upload"})
    """
    return record_provenance_event(
        cx,
        artifact_id=artifact_id,
        event_type="Created",
        author_id=author_id,
        derivation_id=None,
        details=details,
    )


def emit_derived_event(
    cx: psycopg.Connection[Any],
    artifact_id: str,
    derivation_id: str,
    author_id: Optional[str] = None,
    details: Optional[dict[str, Any]] = None,
) -> str:
    """Emit a Derived provenance event for transformed artifacts.
    
    Convenience wrapper for compose/transform derivations.
    
    Args:
        cx: Database connection
        artifact_id: UUID of derived artifact
        derivation_id: UUID of derivation edge
        author_id: Optional author UUID
        details: Optional metadata (e.g., operation, source_count, transformation_params)
        
    Returns:
        Event UUID
        
    Example:
        >>> emit_derived_event(cx, target_id, deriv_id, author_id, {"operation": "compose"})
    """
    return record_provenance_event(
        cx,
        artifact_id=artifact_id,
        event_type="Derived",
        author_id=author_id,
        derivation_id=derivation_id,
        details=details,
    )


def emit_modified_event(
    cx: psycopg.Connection[Any],
    artifact_id: str,
    derivation_id: str,
    author_id: Optional[str] = None,
    details: Optional[dict[str, Any]] = None,
) -> str:

    """Emit a Modified provenance event for delta operations.
    
    Convenience wrapper for delta derivations.
    
    Args:
        cx: Database connection
        artifact_id: UUID of modified artifact
        derivation_id: UUID of delta derivation
        author_id: Optional author UUID
        details: Optional metadata (e.g., delta_size, change_summary, base_artifact_id)
        
    Returns:
        Event UUID
        
    Example:
        >>> emit_modified_event(cx, artifact_id, deriv_id, author_id, {"delta_size": 128})
    """
    return record_provenance_event(
        cx,
        artifact_id=artifact_id,
        event_type="Modified",
        author_id=author_id,
        derivation_id=derivation_id,
        details=details,
    )


def emit_rendered_event(
    cx: psycopg.Connection[Any],
    artifact_id: str,
    derivation_id: str,
    seed: int,
    renderer_version: str,
    variation_id: str,
    author_id: Optional[str] = None,
    additional_details: Optional[dict[str, Any]] = None,
) -> str:
    """Emit a Rendered provenance event with reproducibility metadata.
    
    Enforces required fields for render variations: seed, rendererVersion, variationId.
    
    Mathematical Guarantee:
    - Reproducibility: same (source + seed + rendererVersion) → identical output
    - Fisher Information: I_IKAM(θ) ≥ I_RAG(θ) + Δ_provenance
    
    Args:
        cx: Database connection
        artifact_id: UUID of rendered artifact
        derivation_id: UUID of render derivation
        seed: Deterministic seed for reproducible rendering
        renderer_version: Renderer version string (e.g., "pdflatex-3.141592")
        variation_id: Variation identifier for this render configuration
        author_id: Optional author UUID
        additional_details: Optional extra metadata (e.g., output_size, renderer_type)
        
    Returns:
        Event UUID
        
    Raises:
        ValueError: If required reproducibility fields are missing
        
    Example:
        >>> emit_rendered_event(
        ...     cx, rendered_id, deriv_id,
        ...     seed=42, renderer_version="pdflatex-3.14", variation_id="var-abc123",
        ...     additional_details={"renderer_type": "latex-pdf", "output_size": 4096}
        ... )
        
    References:
        - AGENTS.md: Mathematical soundness for render variations
        - docs/ikam/FISHER_INFORMATION_GAINS.md: Provenance completeness
    """
    # Merge required fields with additional details
    details = {
        "seed": seed,
        "rendererVersion": renderer_version,
        "variationId": variation_id,
    }
    if additional_details:
        details.update(additional_details)
    
    return record_provenance_event(
        cx,
        artifact_id=artifact_id,
        event_type="Rendered",
        author_id=author_id,
        derivation_id=derivation_id,
        details=details,
    )


# ============================================================================
# IR/DSL Operation Storage
# ============================================================================

def insert_op_shape(cx: psycopg.Connection[Any], op_shape: OpShape) -> None:
    """Insert an operation shape (idempotent on shape_hash conflict)."""
    with cx.cursor() as cur:
        cur.execute(
            """
            INSERT INTO ikam_op_shapes (shape_id, shape_hash, ast, arity, op_type)
            VALUES (%s, %s, %s, %s, %s)
            ON CONFLICT (shape_hash) DO NOTHING
            """,
            (
                op_shape.shape_id,
                op_shape.shape_hash,
                _json.dumps(op_shape.ast.model_dump(mode='json')),
                op_shape.arity,
                op_shape.op_type.value,
            ),
        )


def insert_op_instance(cx: psycopg.Connection[Any], op_instance: OpInstance) -> None:
    """Insert an operation instance (idempotent on instance_id conflict)."""
    with cx.cursor() as cur:
        cur.execute(
            """
            INSERT INTO ikam_op_instances (instance_id, shape_id, artifact_id, params, scope, created_at)
            VALUES (%s, %s, %s, %s, %s, %s)
            ON CONFLICT (instance_id) DO NOTHING
            """,
            (
                op_instance.instance_id,
                op_instance.shape_id,
                uuid.UUID(op_instance.artifact_id) if op_instance.artifact_id else None,
                _json.dumps(op_instance.params),
                op_instance.scope,
                _dt.datetime.now(_dt.timezone.utc),
            ),
        )


def get_op_instances_by_artifact(cx: psycopg.Connection[Any], artifact_id: str) -> list[OpInstance]:
    """Retrieve all operation instances for an artifact."""
    with cx.cursor() as cur:
        cur.execute(
            "SELECT instance_id, shape_id, artifact_id, params, scope FROM ikam_op_instances WHERE artifact_id = %s",
            (uuid.UUID(artifact_id),),
        )
        instances = []
        for row in cur.fetchall():
            instances.append(
                OpInstance(
                    instance_id=row["instance_id"],
                    shape_id=row["shape_id"],
                    artifact_id=str(row["artifact_id"]) if row["artifact_id"] else None,
                    params=row["params"],
                    scope=row["scope"],
                )
            )
        return instances


def get_events_by_author(cx: psycopg.Connection[Any], author_id: str) -> list[dict[str, Any]]:
    """Retrieve events for a given author ordered by time."""
    with cx.cursor() as cur:
        cur.execute(
            "SELECT id, artifact_id, derivation_id, event_type, author_id, created_at, details FROM ikam_provenance_events WHERE author_id = %s ORDER BY created_at ASC",
            (uuid.UUID(author_id),),
        )
        rows = cur.fetchall()
        result: list[dict[str, Any]] = []
        for r in rows:
            result.append(
                {
                    "id": str(r["id"]),
                    "artifact_id": str(r["artifact_id"]) if r["artifact_id"] else None,
                    "derivation_id": str(r["derivation_id"]) if r["derivation_id"] else None,
                    "event_type": r["event_type"],
                    "author_id": str(r["author_id"]) if r["author_id"] else None,
                    "created_at": r["created_at"],
                    "details": r["details"],
                }
            )
        return result


def get_recent_events(cx: psycopg.Connection[Any], limit: int = 20) -> list[dict[str, Any]]:
    """Retrieve most recent provenance events (descending by created_at, then by id to guarantee ordering)."""
    with cx.cursor() as cur:
        cur.execute(
            "SELECT id, artifact_id, derivation_id, event_type, author_id, created_at, details FROM ikam_provenance_events ORDER BY created_at DESC, id DESC LIMIT %s",
            (limit,),
        )
        rows = cur.fetchall()
        events: list[dict[str, Any]] = []
        for r in rows:
            events.append(
                {
                    "id": str(r["id"]),
                    "artifact_id": str(r["artifact_id"]) if r["artifact_id"] else None,
                    "derivation_id": str(r["derivation_id"]) if r["derivation_id"] else None,
                    "event_type": r["event_type"],
                    "author_id": str(r["author_id"]) if r["author_id"] else None,
                    "created_at": r["created_at"],
                    "details": r["details"],
                }
            )
        return events


def get_filtered_events(
    cx: psycopg.Connection[Any],
    *,
    artifact_id: Optional[str] = None,
    author_id: Optional[str] = None,
    event_type: Optional[str] = None,
    since: Optional[_dt.datetime] = None,
    until: Optional[_dt.datetime] = None,
    limit: int = 100,
) -> list[dict[str, Any]]:
    """Flexible provenance event query with filtering and time windows.

    Args:
        cx: Database connection
        artifact_id: Filter to specific artifact UUID
        author_id: Filter to specific author UUID
        event_type: Filter to specific event type (Created|Modified|Derived|Rendered)
        since: Earliest created_at (inclusive)
        until: Latest created_at (exclusive)
        limit: Maximum rows to return (default 100)

    Returns:
        List of event dictionaries ordered by created_at ASC, id ASC
    """
    clauses: list[str] = []
    params: list[Any] = []
    
    if artifact_id:
        clauses.append("artifact_id = %s")
        params.append(uuid.UUID(artifact_id))
    if author_id:
        clauses.append("author_id = %s")
        params.append(uuid.UUID(author_id))
    if event_type:
        clauses.append("event_type = %s")
        params.append(event_type)
    if since:
        clauses.append("created_at >= %s")
        params.append(since)
    if until:
        clauses.append("created_at < %s")
        params.append(until)
    
    where_sql = f"WHERE {' AND '.join(clauses)}" if clauses else ""
    sql = (
        "SELECT id, artifact_id, derivation_id, event_type, author_id, created_at, details "
        f"FROM ikam_provenance_events {where_sql} "
        "ORDER BY created_at ASC, id ASC LIMIT %s"
    )
    params.append(limit)
    
    with cx.cursor() as cur:
        cur.execute(cast(Query, sql), params)
        rows = cur.fetchall()
    
    results: list[dict[str, Any]] = []
    for r in rows:
        results.append(
            {
                "id": str(r["id"]),
                "artifact_id": str(r["artifact_id"]) if r["artifact_id"] else None,
                "derivation_id": str(r["derivation_id"]) if r["derivation_id"] else None,
                "event_type": r["event_type"],
                "author_id": str(r["author_id"]) if r["author_id"] else None,
                "created_at": r["created_at"],
                "details": r["details"],
            }
        )
    return results
