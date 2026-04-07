import json
import uuid

import pytest

from modelado.plans import persist_plan, persist_plan_in_scope, plan_to_ikam_artifact


def _example_plan_dict(*, scope_id: str = "scope-123") -> dict:
    return {
        "schema": "narraciones/ikam-plan@1",
        "scope_id": scope_id,
        "title": "Example Plan",
        "goal": "Make planning deterministic",
        "milestones": [
            {"id": "ms-1", "title": "First milestone"},
            {"id": "ms-2", "title": "Second milestone", "depends_on": ["ms-1"]},
        ],
    }


@pytest.mark.skip(reason="persist_plan and insert_artifact deleted in Phase 4b. Awaiting Task 4b.3.")
def test_persist_plan_inserts_fragments_and_artifact_idempotently(db_connection) -> None:
    plan = _example_plan_dict()

    expected_artifact, expected_fragments, expected_root_ref, _milestone_refs = plan_to_ikam_artifact(plan)

    ref1 = persist_plan(db_connection, plan)
    ref2 = persist_plan(db_connection, plan)

    assert ref1 == expected_root_ref
    assert ref2 == expected_root_ref

    artifact_uuid = uuid.UUID(expected_root_ref.artifact_id)

    with db_connection.cursor() as cur:
        for fragment in expected_fragments:
            cur.execute("SELECT count(*) AS n FROM ikam_fragments WHERE id = %s", (fragment.id,))
            assert (cur.fetchone() or {}).get("n") == 1

        cur.execute("SELECT count(*) AS n FROM ikam_artifacts WHERE id = %s", (artifact_uuid,))
        assert (cur.fetchone() or {}).get("n") == 1

        cur.execute(
            """
            SELECT count(*) AS n
            FROM ikam_artifact_fragments
            WHERE artifact_id = %s
            """,
            (artifact_uuid,),
        )
        assert (cur.fetchone() or {}).get("n") == len(expected_fragments)

        cur.execute(
            """
            SELECT fragment_id
            FROM ikam_artifact_fragments
            WHERE artifact_id = %s
            ORDER BY position
            """,
            (artifact_uuid,),
        )
        rows = cur.fetchall() or []
        fragment_ids_in_db = [r[0] if isinstance(r, tuple) else r.get("fragment_id") for r in rows]
        assert fragment_ids_in_db == expected_artifact.fragment_ids


@pytest.mark.skip(reason="persist_plan and insert_artifact deleted in Phase 4b. Awaiting Task 4b.3.")
def test_persist_plan_in_scope_records_scope_membership(db_connection) -> None:
    scope_id = "scope-test-1"
    conversation_id = "conv-test-1"

    with db_connection.cursor() as cur:
        cur.execute(
            """
            INSERT INTO interacciones_scopes (id, conversation_id, metadata)
            VALUES (%s, %s, %s)
            ON CONFLICT (id) DO NOTHING
            """,
            (scope_id, conversation_id, json.dumps({})),
        )

    plan = _example_plan_dict(scope_id=scope_id)

    ref1 = persist_plan_in_scope(db_connection, plan)
    ref2 = persist_plan_in_scope(db_connection, plan)
    assert ref1 == ref2

    with db_connection.cursor() as cur:
        cur.execute(
            """
            SELECT count(*) AS n
            FROM scope_artifacts
            WHERE scope_id = %s AND artifact_id = %s
            """,
            (scope_id, ref1.artifact_id),
        )
        assert (cur.fetchone() or {}).get("n") == 1
