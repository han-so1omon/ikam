import uuid

import pytest

from modelado.plans import persist_plan
from modelado.plans.persistence import persist_plan_amendment_application


def _base_plan() -> dict:
    return {
        "schema": "narraciones/ikam-plan@1",
        "scope_id": "scope-amend-1",
        "title": "Example Plan",
        "goal": "Make planning deterministic",
        "milestones": [
            {"id": "ms-1", "title": "First milestone"},
            {"id": "ms-2", "title": "Second milestone", "depends_on": ["ms-1"]},
        ],
    }


@pytest.mark.skip(reason="persist_plan and insert_artifact deleted in Phase 4b. Awaiting Task 4b.3.")
def test_persist_plan_amendment_application_records_derivation(db_connection) -> None:
    base_plan = _base_plan()
    base_ref = persist_plan(db_connection, base_plan)

    amendment = {
        "schema": "narraciones/ikam-plan-amendment@1",
        "amendment_id": "test-amend-1",
        "proposed_by": "agent:test",
        "rationale_summary": "Update title for clarity.",
        "verification": [{"artifact_id": base_ref.artifact_id, "fragment_id": base_ref.fragment_id}],
        "base_plan": {"artifact_id": base_ref.artifact_id, "fragment_id": base_ref.fragment_id},
        "delta": [{"op": "replace", "path": "/title", "value": "Updated Title"}],
    }

    amendment_ref, amended_ref, derivation_id = persist_plan_amendment_application(
        db_connection,
        base_plan=base_plan,
        amendment=amendment,
    )

    assert amendment_ref.artifact_id
    assert amendment_ref.fragment_id
    assert amended_ref.artifact_id
    assert amended_ref.fragment_id

    with db_connection.cursor() as cur:
        cur.execute(
            "SELECT count(*) AS n FROM ikam_artifacts WHERE id = %s",
            (uuid.UUID(amendment_ref.artifact_id),),
        )
        assert (cur.fetchone() or {}).get("n") == 1

        cur.execute(
            """
            SELECT edge_label, out_id, in_id, properties
            FROM graph_edge_events
            WHERE project_id = %s
            AND properties->>'derivationId' = %s
            AND in_id = %s
            AND edge_label = 'derivation:delta'
            AND op <> 'delete'
            """,
            (getattr(db_connection, "_test_project_id"), derivation_id, amended_ref.artifact_id),
        )
        rows = cur.fetchall()
        assert len(rows) == 2  # base_plan + amendment -> amended_plan

        sample_props = dict((rows[0] or {}).get("properties") or {})
        params = dict(sample_props.get("parameters") or {})
        assert params.get("amendment_artifact_id") == amendment_ref.artifact_id
        assert params.get("amendment_id") == "test-amend-1"
        assert params.get("proposed_by") == "agent:test"
        assert params.get("rationale_summary") == "Update title for clarity."
        assert isinstance(params.get("verification"), list)

        changed_validations = params.get("changed_validations")
        assert isinstance(changed_validations, dict)
        assert isinstance(changed_validations.get("added"), list)
        assert isinstance(changed_validations.get("removed"), list)

        changed_failure_paths = params.get("changed_failure_paths")
        assert isinstance(changed_failure_paths, dict)
        assert isinstance(changed_failure_paths.get("added"), list)
        assert isinstance(changed_failure_paths.get("removed"), list)

        # The edge log is the source of truth; the 2 edges above are the source associations.


@pytest.mark.skip(reason="persist_plan and insert_artifact deleted in Phase 4b. Awaiting Task 4b.3.")
def test_persist_plan_amendment_application_noop_patch_is_allowed(db_connection) -> None:
    base_plan = _base_plan()
    base_ref = persist_plan(db_connection, base_plan)

    amendment = {
        "schema": "narraciones/ikam-plan-amendment@1",
        "base_plan": {"artifact_id": base_ref.artifact_id, "fragment_id": base_ref.fragment_id},
        "delta": [],
    }

    amendment_ref, amended_ref, derivation_id = persist_plan_amendment_application(
        db_connection,
        base_plan=base_plan,
        amendment=amendment,
    )

    assert amendment_ref.artifact_id
    assert amended_ref.artifact_id == base_ref.artifact_id
    assert derivation_id is None

    with db_connection.cursor() as cur:
        cur.execute(
            "SELECT count(*) AS n FROM ikam_artifacts WHERE id = %s",
            (uuid.UUID(amendment_ref.artifact_id),),
        )
        assert (cur.fetchone() or {}).get("n") == 1


@pytest.mark.skip(reason="persist_plan and insert_artifact deleted in Phase 4b. Awaiting Task 4b.3.")
def test_plan_amendment_delta_chain_is_bounded_with_transform_rebase(db_connection) -> None:
    """After 3 delta derivations, the next amendment is recorded as a transform (rebase)."""
    base_plan = _base_plan()
    unique_suffix = str(uuid.uuid4())
    base_plan = dict(base_plan)
    base_plan["scope_id"] = f"scope-amend-{unique_suffix}"
    base_plan["title"] = f"Example Plan {unique_suffix}"
    base_ref = persist_plan(db_connection, base_plan)

    current_plan = dict(base_plan)
    current_ref = base_ref

    # Apply 4 sequential amendments to create a delta chain that would exceed L=3.
    for i in range(1, 5):
        new_title = f"Example Plan v{i}"
        amendment = {
            "schema": "narraciones/ikam-plan-amendment@1",
            "amendment_id": f"test-amend-{i}",
            "proposed_by": "agent:test",
            "rationale_summary": f"Update title to v{i}.",
            "verification": [
                {"artifact_id": current_ref.artifact_id, "fragment_id": current_ref.fragment_id}
            ],
            "base_plan": {"artifact_id": current_ref.artifact_id, "fragment_id": current_ref.fragment_id},
            "delta": [{"op": "replace", "path": "/title", "value": new_title}],
        }

        amendment_ref, amended_ref, derivation_id = persist_plan_amendment_application(
            db_connection,
            base_plan=current_plan,
            amendment=amendment,
        )

        assert derivation_id is not None

        expected_edge_label = "derivation:delta" if i <= 3 else "derivation:transform"
        with db_connection.cursor() as cur:
            cur.execute(
                """
                SELECT edge_label, properties
                FROM graph_edge_events
                WHERE project_id = %s
                  AND properties->>'derivationId' = %s
                  AND in_id = %s
                  AND edge_label = %s
                  AND op <> 'delete'
                """,
                (
                    getattr(db_connection, "_test_project_id"),
                    derivation_id,
                    amended_ref.artifact_id,
                    expected_edge_label,
                ),
            )
            rows = cur.fetchall()
        assert len(rows) == 2

        params = dict((rows[0] or {}).get("properties") or {}).get("parameters") or {}
        assert params.get("amendment_id") == f"test-amend-{i}"
        assert params.get("amendment_artifact_id") == amendment_ref.artifact_id
        if i > 3:
            rebase = params.get("delta_chain_rebase")
            assert isinstance(rebase, dict)
            assert rebase.get("max_length") == 3
            assert rebase.get("policy") == "promote_to_transform"

        current_plan = dict(current_plan)
        current_plan["title"] = new_title
        current_ref = amended_ref
