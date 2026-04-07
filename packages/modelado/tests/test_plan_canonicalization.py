import json

from modelado.plans import Plan, canonicalize_plan_json, plan_artifact_uuid, plan_cas_id


def _example_plan_dict(scope_id: str = "scope-123") -> dict:
    return {
        "schema": "narraciones/ikam-plan@1",
        "scope_id": scope_id,
        "title": "Example Plan",
        "goal": "Make planning deterministic",
        "milestones": [
            {
                "id": "m1",
                "title": "Define schema",
                "description": "",
                "depends_on": [],
                "success_criteria": ["schema exists"],
                "validations": [{"id": "v1", "description": "schema validates"}],
                "failure_paths": [
                    {
                        "id": "f1",
                        "when": "schema validation fails",
                        "action": "propose amendment patch",
                    }
                ],
            }
        ],
    }


def test_canonicalize_plan_json_is_deterministic() -> None:
    plan_dict = _example_plan_dict()

    b1 = canonicalize_plan_json(plan_dict)
    b2 = canonicalize_plan_json(plan_dict)

    assert b1 == b2
    # Sanity: decodes to valid JSON
    json.loads(b1.decode("utf-8"))


def test_canonicalize_plan_json_equivalent_inputs_match() -> None:
    plan_dict = _example_plan_dict()

    plan_model = Plan.model_validate(plan_dict)

    b_from_dict = canonicalize_plan_json(plan_dict)
    b_from_model = canonicalize_plan_json(plan_model)

    assert b_from_dict == b_from_model


def test_plan_ids_are_stable_and_change_with_content() -> None:
    b1 = canonicalize_plan_json(_example_plan_dict(scope_id="scope-123"))
    b2 = canonicalize_plan_json(_example_plan_dict(scope_id="scope-123"))
    b3 = canonicalize_plan_json(_example_plan_dict(scope_id="scope-456"))

    assert plan_cas_id(b1) == plan_cas_id(b2)
    assert plan_artifact_uuid(b1) == plan_artifact_uuid(b2)

    assert plan_cas_id(b1) != plan_cas_id(b3)
    assert plan_artifact_uuid(b1) != plan_artifact_uuid(b3)
