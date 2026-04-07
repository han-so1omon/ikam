import pytest

from modelado.plans.schema import PlanRef, normalize_plan_ref, plan_ref_from_metadata


def test_normalize_plan_ref_accepts_snake_case():
    plan_ref = normalize_plan_ref({"plan_artifact_id": "plan-1", "fragment_id": "frag-1"})
    assert isinstance(plan_ref, PlanRef)
    assert plan_ref.artifact_id == "plan-1"
    assert plan_ref.fragment_id == "frag-1"


def test_plan_ref_from_metadata_optional():
    assert plan_ref_from_metadata({}, required=False) is None


def test_normalize_plan_ref_rejects_legacy():
    with pytest.raises(ValueError, match="Legacy plan:{sha}"):
        normalize_plan_ref({"plan_artifact_id": "plan:deadbeef", "fragment_id": "frag"})


def test_plan_ref_from_metadata_rejects_legacy():
    with pytest.raises(ValueError, match="Legacy plan:{sha}"):
        plan_ref_from_metadata(
            {"plan_ref": {"plan_artifact_id": "plan:deadbeef", "fragment_id": "frag"}},
            required=True,
        )
