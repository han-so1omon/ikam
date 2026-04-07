from modelado.graph_edge_event_log import list_graph_edge_events
from modelado.plans.plan_run_edges import (
    append_derived_from_edges,
    append_fired_transition_edge,
    append_plan_run_of_edge,
)


def _find_event(events, *, edge_label, out_id, in_id):
    for event in events:
        if event.edge_label == edge_label and event.out_id == out_id and event.in_id == in_id:
            return event
    return None


def test_plan_run_edge_event_emission(db_connection):
    project_id = db_connection._test_project_id  # type: ignore[attr-defined]

    plan_artifact_id = "plan-artifact-1"
    plan_fragment_id = "plan-fragment-1"
    plan_run_artifact_id = "plan-run-1"
    run_id = "run-1"
    transition_fragment_id = "transition-frag-1"
    transition_id = "transition-1"
    firing_fragment_id = "firing-frag-1"

    append_plan_run_of_edge(
        db_connection,
        project_id=project_id,
        plan_artifact_id=plan_artifact_id,
        plan_fragment_id=plan_fragment_id,
        plan_run_artifact_id=plan_run_artifact_id,
        run_id=run_id,
    )

    append_fired_transition_edge(
        db_connection,
        project_id=project_id,
        plan_artifact_id=plan_artifact_id,
        plan_run_artifact_id=plan_run_artifact_id,
        transition_fragment_id=transition_fragment_id,
        transition_id=transition_id,
        firing_fragment_id=firing_fragment_id,
    )

    append_derived_from_edges(
        db_connection,
        project_id=project_id,
        plan_artifact_id=plan_artifact_id,
        run_id=run_id,
        firing_fragment_id=firing_fragment_id,
        transition_fragment_id=transition_fragment_id,
        output_artifact_ids=["out-1"],
        input_artifact_ids=["in-1", "in-2"],
    )

    events = list_graph_edge_events(db_connection, project_id=project_id)
    assert len(events) == 4

    plan_run_edge = _find_event(
        events,
        edge_label="derivation:plan_run_of",
        out_id=plan_run_artifact_id,
        in_id=plan_artifact_id,
    )
    assert plan_run_edge is not None
    assert plan_run_edge.properties == {
        "plan_fragment_id": plan_fragment_id,
        "run_id": run_id,
    }

    fired_edge = _find_event(
        events,
        edge_label="derivation:fired_transition",
        out_id=plan_run_artifact_id,
        in_id=plan_artifact_id,
    )
    assert fired_edge is not None
    assert fired_edge.properties == {
        "transition_fragment_id": transition_fragment_id,
        "transition_id": transition_id,
        "firing_fragment_id": firing_fragment_id,
    }

    derived_edges = [
        event
        for event in events
        if event.edge_label == "derivation:derived_from"
        and event.out_id == "out-1"
        and event.in_id in {"in-1", "in-2"}
    ]
    assert len(derived_edges) == 2
    for edge in derived_edges:
        assert edge.properties == {
            "run_id": run_id,
            "plan_artifact_id": plan_artifact_id,
            "firing_fragment_id": firing_fragment_id,
            "transition_fragment_id": transition_fragment_id,
        }
