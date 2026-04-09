from __future__ import annotations

import datetime as dt
import json
from typing import Any, Sequence

from modelado.core.execution_context import ExecutionPolicyViolation, get_execution_context, require_write_scope
from ikam.adapters import (
    build_fragment_object_manifest,
    extract_fragment_ids_from_manifest,
    fragment_object_id_for_manifest,
    serialize_fragment_object_manifest,
)
from modelado.history.ref_head import build_ref_head


class PostgresArtifactStore:
    """Postgres-backed ArtifactStore.

    This adapter is bound to a caller-owned connection/transaction.
    """

    def __init__(self, cx: Any):
        self._cx = cx

    def upsert_artifact_with_fragments(
        self,
        *,
        artifact_id: str,
        kind: str,
        title: str | None,
        created_at: dt.datetime | None,
        fragment_ids: Sequence[str],
        fragments: Sequence[Any] | None = None,
        domain_id_to_cas_id: dict[str, str] | None = None,
        fragment_manifest: dict | None = None,
        project_id: str | None = None,
        status: str | None = None,
    ) -> None:
        ctx = get_execution_context()
        if ctx is None:
            raise ExecutionPolicyViolation(
                "Execution policy violation: 'ikam.upsert_artifact_with_fragments' requires an active execution context"
            )
        if ctx.actor_id is None:
            require_write_scope("ikam.upsert_artifact_with_fragments")

        resolved_project_id = project_id
        if not resolved_project_id and ctx and ctx.write_scope and ctx.write_scope.project_id:
            resolved_project_id = str(ctx.write_scope.project_id)
        if not resolved_project_id:
            test_project_id = getattr(self._cx, "_test_project_id", None)
            if isinstance(test_project_id, str) and test_project_id:
                resolved_project_id = test_project_id
        if not resolved_project_id:
            resolved_project_id = "default-project"

        # Upsert artifact row (keep original created_at if already present)
        self._cx.execute(
            """
            INSERT INTO ikam_artifacts (id, kind, title, project_id, status, created_at, updated_at)
            VALUES (%s::uuid, %s, %s, %s, %s, COALESCE(%s, now()), COALESCE(%s, now()))
            ON CONFLICT (id) DO UPDATE
            SET kind = EXCLUDED.kind,
                title = EXCLUDED.title,
                project_id = COALESCE(EXCLUDED.project_id, ikam_artifacts.project_id),
                status = COALESCE(EXCLUDED.status, ikam_artifacts.status),
                updated_at = COALESCE(EXCLUDED.updated_at, ikam_artifacts.updated_at)
            """,
            (artifact_id, kind, title, resolved_project_id, status, created_at, created_at),
        )

        # Replace membership atomically within the caller's transaction.
        self._cx.execute(
            "DELETE FROM ikam_artifact_fragments WHERE artifact_id = %s::uuid",
            (artifact_id,),
        )

        if not fragment_ids:
            return

        params = [(artifact_id, fragment_id, idx) for idx, fragment_id in enumerate(fragment_ids)]
        self._cx.executemany(
            """
            INSERT INTO ikam_artifact_fragments (artifact_id, fragment_id, position)
            VALUES (%s::uuid, %s, %s)
            """,
            params,
        )

        object_id = self._insert_fragment_object(
            artifact_id=artifact_id,
            kind=kind,
            fragment_ids=fragment_ids,
            fragments=fragments,
            domain_id_to_cas_id=domain_id_to_cas_id,
            fragment_manifest=fragment_manifest,
        )
        self._cx.execute(
            "UPDATE ikam_artifacts SET head_object_id = %s WHERE id = %s::uuid",
            (object_id, artifact_id),
        )

    def get_fragment_ids_for_artifact(self, *, artifact_id: str) -> list[str]:
        manifest = self.get_manifest_for_artifact(artifact_id=artifact_id)
        if not manifest:
            return []
        return extract_fragment_ids_from_manifest(manifest)

    def get_manifest_for_artifact(self, *, artifact_id: str) -> dict | None:
        row = self._cx.execute(
            """
            SELECT o.manifest
              FROM ikam_artifacts a
              JOIN ikam_fragment_objects o
                ON o.object_id = a.head_object_id
             WHERE a.id = %s::uuid
            """,
            (artifact_id,),
        ).fetchone()
        if not row:
            return None
        return row["manifest"] if isinstance(row, dict) else row[0]

    def upsert_artifact_head_ref(
        self,
        *,
        artifact_id: str,
        ref: str,
        head_object_id: str,
        head_commit_id: str,
    ) -> None:
        branch_name = _branch_name_from_ref(ref)
        result_ref = build_ref_head(ref=ref, commit_id=head_commit_id, head_object_id=head_object_id)
        self._cx.execute(
            """
            INSERT INTO ikam_artifact_commits (artifact_id, id, base_ref, result_ref, delta_hash, view_hash, staged_artifact_id)
            VALUES (%s::uuid, %s, %s::jsonb, %s::jsonb, %s, %s, %s::uuid)
            ON CONFLICT (id) DO NOTHING;

            INSERT INTO ikam_artifact_branches (artifact_id, name, head_commit_id, status, created_at, updated_at)
            VALUES (%s::uuid, %s, %s, 'open', now(), now())
            ON CONFLICT (artifact_id, name) DO UPDATE
            SET head_commit_id = EXCLUDED.head_commit_id,
                updated_at = now()
            """,
            (
                artifact_id,
                head_commit_id,
                '{"ref": null}',
                json.dumps(result_ref, sort_keys=True, separators=(",", ":"), ensure_ascii=False),
                head_commit_id,
                head_object_id,
                artifact_id,
                artifact_id,
                branch_name,
                head_commit_id,
            ),
        )

    def _insert_fragment_object(
        self,
        *,
        artifact_id: str,
        kind: str,
        fragment_ids: Sequence[str],
        fragments: Sequence[Any] | None = None,
        domain_id_to_cas_id: dict[str, str] | None = None,
        fragment_manifest: dict | None = None,
    ) -> str:
        if fragment_manifest is None:
            fragment_manifest = build_fragment_object_manifest(
                artifact_id=artifact_id,
                kind=kind,
                fragment_ids=fragment_ids,
            )
        payload = serialize_fragment_object_manifest(fragment_manifest)
        object_id = fragment_object_id_for_manifest(fragment_manifest)
        root_fragment_id = fragment_ids[0]

        self._cx.execute(
            """
            INSERT INTO ikam_fragment_objects (object_id, root_fragment_id, manifest)
            VALUES (%s, %s, %s::jsonb)
            ON CONFLICT (object_id) DO NOTHING
            """,
            (object_id, root_fragment_id, payload.decode("utf-8")),
        )
        return object_id


def _branch_name_from_ref(ref: str) -> str:
    prefix = "refs/heads/"
    if not ref.startswith(prefix):
        raise ValueError(f"Unsupported ref: {ref}")
    branch_name = ref[len(prefix) :]
    if not branch_name:
        raise ValueError(f"Unsupported ref: {ref}")
    return branch_name
