import pytest

from modelado.plans.schema import (
    PetriNetEnvelope,
    PetriNetMarking,
    PetriNetPlace,
    PetriNetTransition,
    PetriNetArcEndpoint,
    PlanRef,
    PetriNetRunEnvelope,
    PlanTransitionRef,
    PetriNetRunFiring,
    canonicalize_petri_net_envelope_json,
    canonicalize_petri_run_firing_json,
    require_non_negative_marking,
    require_transition_place_refs,
    require_unique_place_ids,
    require_unique_transition_ids,
)


def test_petri_net_envelope_canonicalization_is_deterministic():
    envelope = PetriNetEnvelope(
        project_id="proj-1",
        scope_id="scope-1",
        title="Plan",
        goal="Ship",
        place_fragment_ids=["p1"],
        transition_fragment_ids=["t1"],
        arc_fragment_ids=[],
        initial_marking_fragment_id="m1",
    )

    first = canonicalize_petri_net_envelope_json(envelope)
    second = canonicalize_petri_net_envelope_json(envelope.model_dump(by_alias=True))
    assert first == second


def test_petri_net_validation_helpers():
    places = [
        PetriNetPlace(place_id="p1", label="Start"),
        PetriNetPlace(place_id="p2", label="Done"),
    ]
    transitions = [
        PetriNetTransition(
            transition_id="t1",
            label="Do",
            operation_ref="op-1",
            inputs=[PetriNetArcEndpoint(place_id="p1", weight=1)],
            outputs=[PetriNetArcEndpoint(place_id="p2", weight=1)],
        )
    ]

    require_unique_place_ids(places)
    require_unique_transition_ids(transitions)
    require_transition_place_refs(transitions, [p.place_id for p in places])

    with pytest.raises(ValueError, match="Negative marking"):
        require_non_negative_marking(PetriNetMarking(tokens={"p1": -1}))


def test_petri_run_firing_canonicalization():
    firing = PetriNetRunFiring(
        firing_id="f1",
        transition_ref=PlanTransitionRef(
            plan_artifact_id="plan-1",
            transition_fragment_id="tfrag-1",
            transition_id="t1",
        ),
        marking_before_fragment_id="m0",
        marking_after_fragment_id="m1",
        status="success",
        ts_ms=123,
    )
    first = canonicalize_petri_run_firing_json(firing)
    second = canonicalize_petri_run_firing_json(firing.model_dump(by_alias=True))
    assert first == second
