"""Async repository for persisting IKAM graph entities.

This module mirrors a small subset of `modelado.ikam_graph_repository`, but for
`psycopg.AsyncConnection`.

Purpose:
- Keep IKAM writes behind a single chokepoint that enforces `ExecutionContext`
  and strict-fails unsigned system writes via `WriteScope`.

Note:
- Reads are out of scope here; this module focuses on writes.
"""

from __future__ import annotations

import json as _json
import uuid
from typing import Any, Optional

import ikam.adapters as ikam_adapters
import psycopg
from psycopg.types.json import Jsonb

from modelado.core.execution_context import ExecutionPolicyViolation, get_execution_context, require_write_scope


def _require_ikam_write(operation: str) -> None:
    """Enforce execution policy for IKAM write operations (async variant)."""

    ctx = get_execution_context()
    if ctx is None:
        raise ExecutionPolicyViolation(
            f"Execution policy violation: '{operation}' requires an execution context (ctx=None)"
        )

    # Only system actor writes are signature-gated.
    if ctx.actor_id is None:
        require_write_scope(operation)


async def upsert_artifact(
    cx: psycopg.AsyncConnection,
    *,
    artifact_id: str,
    kind: str,
    title: str | None = None,
    root_fragment_id: str | None = None,
) -> None:
    """Ensure an IKAM artifact row exists (idempotent).

    V3 writes should pass ``root_fragment_id`` for DAG reconstruction.
    """

    _require_ikam_write("upsert_artifact")
    async with cx.cursor() as cur:
        await cur.execute(
            """
            INSERT INTO ikam_artifacts (id, kind, title, root_fragment_id)
            VALUES (%s::uuid, %s, %s, %s)
            ON CONFLICT (id) DO NOTHING
            """,
            (artifact_id, kind, title, root_fragment_id),
        )


async def insert_domain_fragment(
    cx: psycopg.AsyncConnection,
    domain_fragment: Any,
    *,
    domain_id_to_cas_id: dict[str, str] | None = None,
) -> None:
    """Insert a domain fragment into CAS storage."""

    _require_ikam_write("insert_domain_fragment")
    storage_fragment = ikam_adapters.v3_to_storage(domain_fragment)
    domain_fragment_id = getattr(domain_fragment, "id", None) or getattr(domain_fragment, "fragment_id", None)
    if domain_id_to_cas_id is not None and domain_fragment_id is not None:
        storage_fragment = storage_fragment.model_copy(
            update={"id": domain_id_to_cas_id.get(str(domain_fragment_id), storage_fragment.id)}
        )

    async with cx.cursor() as cur:
        await cur.execute(
            """
            INSERT INTO ikam_fragments (id, mime_type, size, bytes)
            VALUES (%s, %s, %s, %s)
            ON CONFLICT (id) DO NOTHING
            """,
            (storage_fragment.id, storage_fragment.mime_type, storage_fragment.size, storage_fragment.bytes),
        )


async def insert_domain_fragments(
    cx: psycopg.AsyncConnection,
    fragments: list[Any],
) -> dict[str, str]:
    """Insert a set of domain fragments while preserving hierarchy ordering."""

    _require_ikam_write("insert_domain_fragments")
    if not fragments:
        return {}

    domain_id_to_cas_id = {
        str(fragment_id): ikam_adapters.v3_to_storage(frag).id
        for frag in fragments
        for fragment_id in [getattr(frag, "id", None) or getattr(frag, "fragment_id", None)]
        if fragment_id is not None
    }

    for frag in sorted(fragments, key=lambda f: getattr(f, "level", 0)):
        await insert_domain_fragment(cx, frag, domain_id_to_cas_id=domain_id_to_cas_id)

    return domain_id_to_cas_id


async def link_artifact_fragment(
    cx: psycopg.AsyncConnection,
    *,
    artifact_id: str,
    fragment_id: str,
    position: int,
) -> None:
    """Link a fragment to an artifact in `ikam_artifact_fragments` (idempotent)."""

    _require_ikam_write("link_artifact_fragment")
    async with cx.cursor() as cur:
        await cur.execute(
            """
            INSERT INTO ikam_artifact_fragments (artifact_id, fragment_id, position)
            VALUES (%s::uuid, %s, %s)
            ON CONFLICT (artifact_id, fragment_id) DO NOTHING
            """,
            (artifact_id, fragment_id, position),
        )


async def record_provenance_event(
    cx: psycopg.AsyncConnection,
    *,
    artifact_id: str,
    event_type: str,
    author_id: Optional[str],
    details: dict[str, Any] | None = None,
    derivation_id: Optional[str] = None,
) -> str:
    """Append a provenance event row (idempotency handled by caller if needed)."""

    _require_ikam_write("record_provenance_event")

    event_id = str(uuid.uuid4())
    async with cx.cursor() as cur:
        await cur.execute(
            """
            INSERT INTO ikam_provenance_events (id, artifact_id, derivation_id, event_type, author_id, details)
            VALUES (%s::uuid, %s::uuid, %s::uuid, %s, %s::uuid, %s)
            """,
            (
                event_id,
                artifact_id,
                derivation_id,
                event_type,
                author_id,
                Jsonb(details or {}),
            ),
        )

    return event_id
