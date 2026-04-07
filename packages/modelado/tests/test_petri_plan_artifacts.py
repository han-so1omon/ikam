import pytest

from modelado.plans.artifacts import petri_net_to_ikam_artifact
from modelado.plans.schema import PetriNetMarking, PetriNetPlace, PetriNetTransition, PetriNetArcEndpoint


def test_petri_net_to_ikam_artifact_stable_plan_ref():
    places = [
        PetriNetPlace(place_id="start", label="Start"),
        PetriNetPlace(place_id="done", label="Done"),
    ]
    transitions = [
        PetriNetTransition(
            transition_id="t1",
            label="Step",
            operation_ref="op-1",
            inputs=[PetriNetArcEndpoint(place_id="start", weight=1)],
            outputs=[PetriNetArcEndpoint(place_id="done", weight=1)],
        )
    ]
    marking = PetriNetMarking(tokens={"start": 1})

    artifact, fragments, plan_ref, transition_refs = petri_net_to_ikam_artifact(
        project_id="proj-1",
        scope_id="scope-1",
        title="Demo",
        goal="Ship",
        places=places,
        transitions=transitions,
        arcs=[],
        marking=marking,
    )

    envelope_fragment = fragments[0]
    assert plan_ref.fragment_id == envelope_fragment.id
    assert plan_ref.artifact_id == artifact.id
    assert transition_refs["t1"].artifact_id == artifact.id



def test_petri_net_validation_rejects_bad_marking():
    places = [PetriNetPlace(place_id="p1", label="Start")]
    transitions = []
    marking = PetriNetMarking(tokens={"p1": -1})

    with pytest.raises(ValueError, match="Negative marking"):
        petri_net_to_ikam_artifact(
            project_id="proj-1",
            scope_id="scope-1",
            title="Demo",
            goal="Ship",
            places=places,
            transitions=transitions,
            arcs=[],
            marking=marking,
        )
