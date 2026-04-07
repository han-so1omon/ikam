import pytest

from modelado.plans import (
    apply_plan_amendment,
    apply_plan_amendments,
    apply_plan_patch,
    amendment_artifact_uuid,
    canonicalize_plan_json,
    canonicalize_plan_amendment_json,
    plan_to_ikam_artifact,
)


def _base_plan() -> dict:
    return {
        "schema": "narraciones/ikam-plan@1",
        "scope_id": "scope-123",
        "title": "Example Plan",
        "goal": "Make planning deterministic",
        "milestones": [
            {"id": "ms-1", "title": "First milestone"},
            {"id": "ms-2", "title": "Second milestone", "depends_on": ["ms-1"]},
        ],
    }


def test_apply_plan_amendment_replace_title_is_deterministic() -> None:
    plan = _base_plan()
    artifact, fragments, root_ref, _milestone_refs = plan_to_ikam_artifact(plan)

    amendment = {
        "schema": "narraciones/ikam-plan-amendment@1",
        "base_plan": {"artifact_id": root_ref.artifact_id, "fragment_id": root_ref.fragment_id},
        "delta": [{"op": "replace", "path": "/title", "value": "Updated Title"}],
    }

    amended1 = apply_plan_amendment(plan, amendment)
    amended2 = apply_plan_amendment(plan, amendment)

    assert canonicalize_plan_json(amended1) == canonicalize_plan_json(amended2)
    assert amended1.title == "Updated Title"


def test_apply_plan_amendment_add_milestone_append() -> None:
    plan = _base_plan()
    artifact, fragments, root_ref, _milestone_refs = plan_to_ikam_artifact(plan)

    amendment = {
        "schema": "narraciones/ikam-plan-amendment@1",
        "base_plan": {"artifact_id": root_ref.artifact_id, "fragment_id": root_ref.fragment_id},
        "delta": [
            {
                "op": "add",
                "path": "/milestones/-",
                "value": {"id": "ms-3", "title": "Third milestone", "depends_on": ["ms-2"]},
            }
        ],
    }

    amended = apply_plan_amendment(plan, amendment)
    assert [m.id for m in amended.milestones] == ["ms-1", "ms-2", "ms-3"]


def test_apply_plan_amendment_remove_field() -> None:
    plan = _base_plan()
    artifact, fragments, root_ref, _milestone_refs = plan_to_ikam_artifact(plan)

    amendment = {
        "schema": "narraciones/ikam-plan-amendment@1",
        "base_plan": {"artifact_id": root_ref.artifact_id, "fragment_id": root_ref.fragment_id},
        "delta": [{"op": "remove", "path": "/milestones/0/depends_on"}],
    }

    amended = apply_plan_amendment(plan, amendment)
    assert amended.milestones[0].depends_on == []


def test_apply_plan_amendment_replace_missing_path_raises() -> None:
    plan = _base_plan()
    artifact, fragments, root_ref, _milestone_refs = plan_to_ikam_artifact(plan)

    amendment = {
        "schema": "narraciones/ikam-plan-amendment@1",
        "base_plan": {"artifact_id": root_ref.artifact_id, "fragment_id": root_ref.fragment_id},
        "delta": [{"op": "replace", "path": "/does_not_exist", "value": 1}],
    }

    with pytest.raises(ValueError, match="Path does not exist"):
        apply_plan_amendment(plan, amendment)


def test_apply_plan_amendments_is_deterministic_regardless_of_arrival_order() -> None:
    plan = _base_plan()
    _artifact, _fragments, root_ref, _milestone_refs = plan_to_ikam_artifact(plan)

    amend_title = {
        "schema": "narraciones/ikam-plan-amendment@1",
        "amendment_id": "a-1",
        "proposed_by": "agent:test",
        "rationale_summary": "Rename for clarity.",
        "verification": [{"artifact_id": root_ref.artifact_id, "fragment_id": root_ref.fragment_id}],
        "base_plan": {"artifact_id": root_ref.artifact_id, "fragment_id": root_ref.fragment_id},
        "delta": [{"op": "replace", "path": "/title", "value": "Updated Title"}],
    }

    amend_add_ms = {
        "schema": "narraciones/ikam-plan-amendment@1",
        "amendment_id": "a-2",
        "base_plan": {"artifact_id": root_ref.artifact_id, "fragment_id": root_ref.fragment_id},
        "delta": [
            {
                "op": "add",
                "path": "/milestones/-",
                "value": {"id": "ms-3", "title": "Third milestone", "depends_on": ["ms-2"]},
            }
        ],
    }

    amended_a = apply_plan_amendments(plan, [amend_add_ms, amend_title])
    amended_b = apply_plan_amendments(plan, [amend_title, amend_add_ms])

    assert canonicalize_plan_json(amended_a) == canonicalize_plan_json(amended_b)
    assert amended_a.title == "Updated Title"
    assert [m.id for m in amended_a.milestones] == ["ms-1", "ms-2", "ms-3"]


def test_apply_plan_patch_add_inserts_into_arrays() -> None:
    doc = {"arr": [1, 2, 3]}
    patched = apply_plan_patch(doc, [{"op": "add", "path": "/arr/1", "value": 9}])
    assert patched["arr"] == [1, 9, 2, 3]


def test_apply_plan_patch_replace_requires_existing_array_index() -> None:
    doc = {"arr": [1, 2, 3]}
    patched = apply_plan_patch(doc, [{"op": "replace", "path": "/arr/1", "value": 9}])
    assert patched["arr"] == [1, 9, 3]

    with pytest.raises(ValueError, match="Array index out of range"):
        apply_plan_patch(doc, [{"op": "replace", "path": "/arr/3", "value": 9}])


def test_apply_plan_patch_dash_token_only_allowed_for_add() -> None:
    doc = {"arr": [1, 2]}
    patched = apply_plan_patch(doc, [{"op": "add", "path": "/arr/-", "value": 3}])
    assert patched["arr"] == [1, 2, 3]

    with pytest.raises(ValueError, match="Path does not exist"):
        apply_plan_patch(doc, [{"op": "replace", "path": "/arr/-", "value": 9}])

    with pytest.raises(ValueError, match="Invalid array index token"):
        apply_plan_patch(doc, [{"op": "remove", "path": "/arr/-"}])


def test_apply_plan_patch_unescapes_json_pointer_tokens() -> None:
    doc = {"a/b": 1, "c~d": 2}
    patched = apply_plan_patch(doc, [{"op": "replace", "path": "/a~1b", "value": 10}])
    assert patched["a/b"] == 10

    patched2 = apply_plan_patch(doc, [{"op": "replace", "path": "/c~0d", "value": 20}])
    assert patched2["c~d"] == 20


def test_apply_plan_patch_requires_value_for_replace() -> None:
    doc = {"x": 1}
    with pytest.raises(ValueError, match="value is required"):
        apply_plan_patch(doc, [{"op": "replace", "path": "/x"}])


def test_canonicalize_plan_amendment_json_is_stable_and_uuid_is_deterministic() -> None:
    amendment_a = {
        "schema": "narraciones/ikam-plan-amendment@1",
        "base_plan": {"artifact_id": "a", "fragment_id": "f"},
        "delta": [{"op": "replace", "path": "/title", "value": "Updated"}],
        "amendment_id": None,
    }

    amendment_b = {
        "delta": [{"path": "/title", "value": "Updated", "op": "replace"}],
        "base_plan": {"fragment_id": "f", "artifact_id": "a"},
        "schema": "narraciones/ikam-plan-amendment@1",
    }

    bytes_a = canonicalize_plan_amendment_json(amendment_a)
    bytes_b = canonicalize_plan_amendment_json(amendment_b)
    assert bytes_a == bytes_b

    uuid_a = amendment_artifact_uuid(bytes_a)
    uuid_b = amendment_artifact_uuid(bytes_b)
    assert uuid_a == uuid_b

    amendment_c = {
        **amendment_b,
        "delta": [{"op": "replace", "path": "/title", "value": "Different"}],
    }
    bytes_c = canonicalize_plan_amendment_json(amendment_c)
    assert amendment_artifact_uuid(bytes_c) != uuid_a
