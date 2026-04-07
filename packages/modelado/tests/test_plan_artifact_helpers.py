import datetime as dt

from modelado.plans import canonicalize_plan_json, plan_artifact_uuid, plan_to_ikam_artifact, reconstruct_plan_from_fragments


def _example_plan_dict() -> dict:
    return {
        "schema": "narraciones/ikam-plan@1",
        "scope_id": "scope-123",
        "title": "Example Plan",
        "goal": "Make planning deterministic",
        "milestones": [
            {
                "id": "ms-1",
                "title": "First milestone",
                "validations": [{"id": "v-1", "description": "It works"}],
                "failure_paths": [
                    {"id": "fp-1", "when": "it fails", "action": "amend and retry"}
                ],
            }
        ],
    }


def test_plan_to_ikam_artifact_is_stable() -> None:
    plan = _example_plan_dict()
    fixed_time = dt.datetime(2020, 1, 1, tzinfo=dt.timezone.utc)

    artifact1, fragments1, root_ref1, milestone_refs1 = plan_to_ikam_artifact(plan, created_at=fixed_time)
    artifact2, fragments2, root_ref2, milestone_refs2 = plan_to_ikam_artifact(plan, created_at=fixed_time)

    assert artifact1 == artifact2
    assert fragments1 == fragments2
    assert root_ref1 == root_ref2
    assert milestone_refs1 == milestone_refs2

    assert artifact1.kind == "file"

    assert root_ref1.fragment_id == fragments1[0].id

    assert artifact1.id == str(plan_artifact_uuid(canonicalize_plan_json(plan)))

    envelope_fragment = fragments1[0]
    milestone_fragments = fragments1[1:]
    reconstructed = reconstruct_plan_from_fragments(
        envelope_fragment=envelope_fragment,
        milestone_fragments=milestone_fragments,
    )
    assert canonicalize_plan_json(reconstructed) == canonicalize_plan_json(plan)


def test_plan_to_ikam_artifact_matches_canonical_bytes_and_uuid() -> None:
    plan = _example_plan_dict()

    plan_bytes = canonicalize_plan_json(plan)
    expected_uuid = str(plan_artifact_uuid(plan_bytes))

    artifact, fragments, _root_ref, _milestone_refs = plan_to_ikam_artifact(
        plan,
        created_at=dt.datetime(2020, 1, 1, tzinfo=dt.timezone.utc),
    )

    reconstructed = reconstruct_plan_from_fragments(
        envelope_fragment=fragments[0],
        milestone_fragments=fragments[1:],
    )
    assert canonicalize_plan_json(reconstructed) == plan_bytes
    assert artifact.id == expected_uuid
