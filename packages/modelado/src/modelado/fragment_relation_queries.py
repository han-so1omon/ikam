"""Legacy compatibility helpers for pre-V3 fragment relation queries.

This module depends on the legacy ``ikam_fragment_meta`` and
``ikam_fragment_content`` tables plus the older relation payload shape.
It does not implement the V3 fragment boundary and should not be presented
as a V3-aligned query path.

Keep this module isolated for compatibility-only callers. Do not add new V3
runtime or repository callers here.

Legacy usage:
    from narraciones_base_api.app.core.database import get_connection
    from modelado.fragment_relation_queries import FragmentRelationQuery

    with get_connection() as cx:
        q = FragmentRelationQuery(cx)
        edges = q.list_relations(project_id, predicate="depends_on")
"""

from __future__ import annotations

import json
import logging
from typing import Any, Dict, List, Optional, cast
from uuid import UUID

import psycopg
from psycopg.abc import Query

logger = logging.getLogger(__name__)


def _row_value(row: Any, key: str, idx: int) -> Any:
    if row is None:
        return None
    try:
        return row[key]
    except Exception:
        try:
            return row.get(key)
        except Exception:
            try:
                return row[idx]
            except Exception:
                return None


def _row_mapping(row: Any, columns: list[str]) -> Dict[str, Any]:
    if isinstance(row, dict):
        return dict(row)
    try:
        return {column: row[column] for column in columns}
    except Exception:
        pass
    try:
        return dict(row)
    except Exception:
        return {column: row[idx] for idx, column in enumerate(columns) if idx < len(row)}


class FragmentRelationQuery:
    """Legacy compatibility query helper over pre-V3 relation tables."""

    def __init__(self, connection: psycopg.Connection):
        self.conn = connection

    def list_relations(
        self,
        project_id: UUID | str,
        *,
        predicate: str | None = None,
        subject_fragment_id: str | None = None,
        object_fragment_id: str | None = None,
        limit: int = 200,
        offset: int = 0,
    ) -> List[Dict[str, Any]]:
        """List relation fragments, optionally filtered by predicate/subject/object."""

        sql = """
            SELECT
                fm.fragment_id as id,
                fm.artifact_id as project_id,
                fm.created_at,
                fm.salience,
                fc.content->>'predicate' as predicate,
                fc.content->'subject_fragment_ids' as subject_fragment_ids,
                fc.content->'object_fragment_ids' as object_fragment_ids,
                COALESCE((fc.content->>'directed')::boolean, true) as directed,
                COALESCE((fc.content->>'confidence_score')::double precision, 1.0) as confidence,
                COALESCE(fc.content->'qualifiers', '{}'::jsonb) as qualifiers
            FROM ikam_fragment_meta fm
            JOIN ikam_fragment_content fc ON fm.fragment_id = fc.fragment_id
            WHERE fm.type = 'relation'
              AND fm.artifact_id = %s
        """

        params: List[Any] = [str(project_id)]

        if predicate:
            sql += " AND fc.content->>'predicate' = %s"
            params.append(predicate)

        if subject_fragment_id:
            sql += " AND fc.content->'subject_fragment_ids' @> %s::jsonb"
            params.append(json.dumps([subject_fragment_id]))

        if object_fragment_id:
            sql += " AND fc.content->'object_fragment_ids' @> %s::jsonb"
            params.append(json.dumps([object_fragment_id]))

        sql += " ORDER BY fm.created_at DESC LIMIT %s OFFSET %s"
        params.extend([limit, offset])

        with self.conn.cursor() as cur:
            table_row = cur.execute(
                """
                SELECT count(*) AS present
                FROM pg_catalog.pg_class c
                JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
                WHERE n.nspname = current_schema()
                  AND c.relkind IN ('r', 'p')
                  AND c.relname IN ('ikam_fragment_meta', 'ikam_fragment_content')
                """,
            ).fetchone()
        if not table_row or int(_row_value(table_row, "present", 0) or 0) < 2:
            logger.warning(
                "Relation fragment tables not available; returning empty results.",
            )
            return []

        rows = self.conn.execute(cast(Query, sql), params).fetchall()
        columns = [
            "id",
            "project_id",
            "created_at",
            "salience",
            "predicate",
            "subject_fragment_ids",
            "object_fragment_ids",
            "directed",
            "confidence",
            "qualifiers",
        ]
        return [self._row_to_relation(_row_mapping(row, columns)) for row in rows] if rows else []

    def for_subject(
        self,
        subject_fragment_id: str,
        *,
        project_id: UUID | str | None = None,
        predicate: str | None = None,
        limit: int = 200,
    ) -> List[Dict[str, Any]]:
        """Convenience wrapper to list relations where subject contains a given fragment."""

        if not project_id:
            # Avoid expensive inference. Callers should pass project_id explicitly.
            raise ValueError("project_id is required")

        return self.list_relations(
            project_id,
            predicate=predicate,
            subject_fragment_id=subject_fragment_id,
            limit=limit,
            offset=0,
        )

    def for_object(
        self,
        object_fragment_id: str,
        *,
        project_id: UUID | str | None = None,
        predicate: str | None = None,
        limit: int = 200,
    ) -> List[Dict[str, Any]]:
        """Convenience wrapper to list relations where object contains a given fragment."""

        if not project_id:
            raise ValueError("project_id is required")

        return self.list_relations(
            project_id,
            predicate=predicate,
            object_fragment_id=object_fragment_id,
            limit=limit,
            offset=0,
        )

    @staticmethod
    def _row_to_relation(row: Dict[str, Any]) -> Dict[str, Any]:
        # psycopg may return JSONB as str (depending on adapters). Normalize.
        for key in ("subject_fragment_ids", "object_fragment_ids", "qualifiers"):
            val = row.get(key)
            if isinstance(val, str):
                try:
                    row[key] = json.loads(val)
                except Exception:
                    pass
        return row
