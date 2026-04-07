import pytest

from modelado.plans.amendments import apply_petri_net_amendment, validate_petri_plan_patch_ops
from modelado.plans.schema import PlanAmendment, PetriNetEnvelope, PlanPatchOp, PlanRef


def test_petri_plan_patch_allows_envelope_fields():
    validate_petri_plan_patch_ops(
        [
            {"op": "replace", "path": "/title", "value": "New Title"},
            {"op": "add", "path": "/transition_fragment_ids/0", "value": "frag-1"},
            {"op": "replace", "path": "/initial_marking_fragment_id", "value": "mark-1"},
        ]
    )


def test_petri_plan_patch_rejects_non_envelope_fields():
    with pytest.raises(ValueError, match="not allowed"):
        validate_petri_plan_patch_ops(
            [
                {
                    "op": "replace",
                    "path": "/transitions/0/label",
                    "value": "Bad",
                }
            ]
        )


def test_apply_petri_net_amendment_updates_title():
    envelope = PetriNetEnvelope(
        project_id="proj-1",
        scope_id="scope-1",
        title="Old",
        goal="Ship",
        place_fragment_ids=[],
        transition_fragment_ids=[],
        arc_fragment_ids=[],
        initial_marking_fragment_id="m1",
    )
    amendment = PlanAmendment(
        base_plan=PlanRef(artifact_id="plan-1", fragment_id="frag-1"),
        delta=[PlanPatchOp(op="replace", path="/title", value="New")],
    )

    updated = apply_petri_net_amendment(envelope, amendment)
    assert updated.title == "New"
