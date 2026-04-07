"""Async repository for persisting IKAM graph entities via SQLAlchemy AsyncSession.

Purpose:
- Keep IKAM writes behind a single chokepoint.
- Enforce `ExecutionContext` and strict-fail unsigned system writes via `WriteScope`.

This mirrors a subset of `modelado.ikam_graph_repository_async` but operates on
`sqlalchemy.ext.asyncio.AsyncSession` (used by some storage adapters).
"""

from __future__ import annotations

import json as _json
import uuid
from datetime import datetime
from typing import Any, Optional

from sqlalchemy import text
from sqlalchemy.exc import IntegrityError
from sqlalchemy.ext.asyncio import AsyncSession

from modelado.core.execution_context import ExecutionPolicyViolation, get_execution_context, require_write_scope


def _require_ikam_write(operation: str) -> None:
    """Enforce execution policy for IKAM write operations (SQLAlchemy async variant)."""

    ctx = get_execution_context()
    if ctx is None:
        raise ExecutionPolicyViolation(
            f"Execution policy violation: '{operation}' requires an execution context (ctx=None)"
        )

    # Only system actor writes are signature-gated.
    if ctx.actor_id is None:
        require_write_scope(operation)


async def insert_cas_fragment(
    session: AsyncSession,
    *,
    fragment_id: str,
    mime_type: str,
    size: int,
    data: bytes,
) -> None:
    """Insert CAS fragment bytes (idempotent)."""

    _require_ikam_write("insert_cas_fragment")
    await session.execute(
        text(
            """
            INSERT INTO ikam_fragments (id, mime_type, size, bytes)
            VALUES (:id, :mime_type, :size, :bytes)
            ON CONFLICT (id) DO NOTHING
            """
        ),
        {
            "id": fragment_id,
            "mime_type": mime_type,
            "size": size,
            "bytes": data,
        },
    )


async def upsert_artifact(
    session: AsyncSession,
    *,
    artifact_id: uuid.UUID,
    kind: str,
    title: Optional[str],
) -> None:
    """Insert or update an IKAM artifact row."""

    _require_ikam_write("upsert_artifact")
    await session.execute(
        text(
            """
            INSERT INTO ikam_artifacts (id, kind, title)
            VALUES (:id, :kind, :title)
            ON CONFLICT (id) DO UPDATE SET kind = EXCLUDED.kind, title = EXCLUDED.title
            """
        ),
        {"id": artifact_id, "kind": kind, "title": title},
    )


async def link_artifact_fragment(
    session: AsyncSession,
    *,
    artifact_id: uuid.UUID,
    fragment_id: str,
    position: int,
) -> None:
    """Link a fragment to an artifact in `ikam_artifact_fragments` (idempotent)."""

    _require_ikam_write("link_artifact_fragment")
    await session.execute(
        text(
            """
            INSERT INTO ikam_artifact_fragments (artifact_id, fragment_id, position)
            VALUES (:artifact_id, :fragment_id, :position)
            ON CONFLICT (artifact_id, fragment_id) DO NOTHING
            """
        ),
        {"artifact_id": artifact_id, "fragment_id": fragment_id, "position": position},
    )


async def insert_fragment_meta_if_missing(
    session: AsyncSession,
    *,
    fragment_id: str,
    artifact_id: uuid.UUID,
    level: int,
    fragment_type: str,
    parent_fragment_id: Optional[str],
    salience: float,
    created_at: datetime,
    updated_at: datetime,
) -> None:
    """Insert fragment metadata row if missing (idempotent)."""

    _require_ikam_write("insert_fragment_meta_if_missing")
    await session.execute(
        text(
            """
            INSERT INTO ikam_fragment_meta (
                fragment_id, artifact_id, level, type, parent_fragment_id, salience, created_at, updated_at
            )
            VALUES (
                :fragment_id, :artifact_id, :level, :type, :parent_fragment_id, :salience, :created_at, :updated_at
            )
            ON CONFLICT (fragment_id) DO NOTHING
            """
        ),
        {
            "fragment_id": fragment_id,
            "artifact_id": artifact_id,
            "level": int(level),
            "type": fragment_type,
            "parent_fragment_id": parent_fragment_id,
            "salience": salience,
            "created_at": created_at,
            "updated_at": updated_at,
        },
    )


async def backfill_parent_pointer_if_missing(
    session: AsyncSession,
    *,
    artifact_id: uuid.UUID,
    fragment_id: str,
    parent_fragment_id: str,
    updated_at: datetime,
) -> None:
    """Backfill parent pointer when a parent can now be resolved."""

    _require_ikam_write("backfill_parent_pointer_if_missing")
    await session.execute(
        text(
            """
            UPDATE ikam_fragment_meta
            SET parent_fragment_id = :parent_fragment_id, updated_at = :updated_at
            WHERE fragment_id = :fragment_id
              AND artifact_id = :artifact_id
              AND parent_fragment_id IS NULL
            """
        ),
        {
            "parent_fragment_id": parent_fragment_id,
            "updated_at": updated_at,
            "fragment_id": fragment_id,
            "artifact_id": artifact_id,
        },
    )


async def backfill_children_parent_links(
    session: AsyncSession,
    *,
    artifact_id: uuid.UUID,
    concept_id: str,
    parent_fragment_id: str,
    updated_at: datetime,
) -> None:
    """Backfill children created before their parent (idempotent)."""

    _require_ikam_write("backfill_children_parent_links")
    await session.execute(
        text(
            """
            UPDATE ikam_fragment_meta m
            SET parent_fragment_id = :parent_fragment_id, updated_at = :updated_at
            FROM ikam_fragment_content c
            WHERE m.fragment_id = c.fragment_id
              AND m.artifact_id = :artifact_id
              AND m.type = 'concept'
              AND c.content_type = 'concept'
              AND (c.content->>'parent_concept_id') = :concept_id
              AND m.parent_fragment_id IS NULL
              AND m.fragment_id <> :parent_fragment_id
            """
        ),
        {
            "parent_fragment_id": parent_fragment_id,
            "updated_at": updated_at,
            "artifact_id": artifact_id,
            "concept_id": concept_id,
        },
    )


async def insert_fragment_content_if_missing(
    session: AsyncSession,
    *,
    fragment_id: str,
    content_type: str,
    content_json: str,
) -> None:
    """Insert fragment content row if missing (idempotent)."""

    _require_ikam_write("insert_fragment_content_if_missing")
    await session.execute(
        text(
            """
            INSERT INTO ikam_fragment_content (fragment_id, content_type, content)
            VALUES (:fragment_id, :content_type, CAST(:content AS JSONB))
            ON CONFLICT (fragment_id) DO NOTHING
            """
        ),
        {"fragment_id": fragment_id, "content_type": content_type, "content": content_json},
    )


async def record_provenance_event(
    session: AsyncSession,
    *,
    event_id: uuid.UUID,
    artifact_id: uuid.UUID,
    derivation_id: Optional[uuid.UUID],
    event_type: str,
    author_id: Optional[uuid.UUID],
    created_at: datetime,
    details: dict[str, Any],
) -> None:
    """Append a provenance event row."""

    _require_ikam_write("record_provenance_event")
    await session.execute(
        text(
            """
            INSERT INTO ikam_provenance_events (
                id, artifact_id, derivation_id, event_type, author_id, created_at, details
            )
            VALUES (
                :id, :artifact_id, :derivation_id, :event_type, :author_id, :created_at, CAST(:details AS JSONB)
            )
            """
        ),
        {
            "id": event_id,
            "artifact_id": artifact_id,
            "derivation_id": derivation_id,
            "event_type": event_type,
            "author_id": author_id,
            "created_at": created_at,
            "details": _json.dumps(details),
        },
    )


async def record_provenance_event_ignore_unique_violation(
    session: AsyncSession,
    *,
    event_id: uuid.UUID,
    artifact_id: uuid.UUID,
    event_type: str,
    author_id: Optional[uuid.UUID],
    created_at: datetime,
    details: dict[str, Any],
) -> None:
    """Append a provenance event, treating the repo's unique constraint as idempotent."""

    _require_ikam_write("record_provenance_event_ignore_unique_violation")
    try:
        async with session.begin_nested():
            await session.execute(
                text(
                    """
                    INSERT INTO ikam_provenance_events (
                        id, artifact_id, derivation_id, event_type, author_id, created_at, details
                    )
                    VALUES (
                        :id, :artifact_id, NULL, :event_type, :author_id, :created_at, CAST(:details AS JSONB)
                    )
                    """
                ),
                {
                    "id": event_id,
                    "artifact_id": artifact_id,
                    "event_type": event_type,
                    "author_id": author_id,
                    "created_at": created_at,
                    "details": _json.dumps(details),
                },
            )
    except IntegrityError as exc:
        if "ikam_provenance_events_unique_idx" not in str(exc):
            raise
