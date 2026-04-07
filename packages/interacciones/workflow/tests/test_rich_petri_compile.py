"""Tests for richer Petri workflow compilation."""

from __future__ import annotations

import pytest

from interacciones.schemas import ResolutionMode, RichPetriWorkflow, WorkflowDefinition, WorkflowLink, WorkflowNode
from interacciones.workflow import compile_workflow_definition, compile_workflow_to_rich_petri


def test_compile_workflow_definition_returns_rich_petri_workflow() -> None:
    workflow = WorkflowDefinition(
        workflow_id="ingestion-early-steps",
        version="2026-03-06",
        nodes=[
            WorkflowNode(
                node_id="dispatch-normalize",
                kind="dispatch_executor",
                capability="python.transform",
                policy={"cost_tier": "standard"},
                constraints={"locality": "local"},
            ),
            WorkflowNode(node_id="wait-normalize", kind="wait_for_result"),
            WorkflowNode(node_id="complete", kind="complete"),
        ],
        links=[
            WorkflowLink(source="dispatch-normalize", target="wait-normalize"),
            WorkflowLink(source="wait-normalize", target="complete"),
        ],
    )

    compiled = compile_workflow_definition(workflow)

    assert isinstance(compiled, RichPetriWorkflow)
    assert compiled.workflow_id == "ingestion-early-steps"
    assert [place.place_id for place in compiled.places] == [
        "place:start:dispatch-normalize",
        "place:link:dispatch-normalize:wait-normalize",
        "place:link:wait-normalize:complete",
        "place:end:complete",
    ]
    assert [transition.transition_id for transition in compiled.transitions] == [
        "transition:dispatch-normalize",
        "transition:wait-normalize",
        "transition:complete",
    ]
    assert compiled.transitions[0].capability == "python.transform"
    assert compiled.transitions[0].policy == {"cost_tier": "standard"}
    assert compiled.transitions[0].constraints == {"locality": "local"}
    assert compiled.source_workflow_storage.mode.value == "default_on"
    assert compiled.source_workflow_definition.model_dump(mode="json") == workflow.model_dump(mode="json")


def test_compile_workflow_to_rich_petri_preserves_override_and_hints() -> None:
    workflow = WorkflowDefinition(
        workflow_id="approval-flow",
        version="v1",
        nodes=[
            WorkflowNode(
                node_id="dispatch-embed",
                kind="dispatch_executor",
                capability="ml.embed",
                resolution_mode=ResolutionMode.DIRECT_EXECUTOR_REF,
                direct_executor_ref="executor://ml-primary",
            ),
            WorkflowNode(node_id="request-review", kind="request_approval"),
        ],
        links=[WorkflowLink(source="dispatch-embed", target="request-review")],
    )

    compiled = compile_workflow_to_rich_petri(workflow)

    assert compiled.transitions[0].resolution_mode is ResolutionMode.DIRECT_EXECUTOR_REF
    assert compiled.transitions[0].direct_executor_ref == "executor://ml-primary"
    assert compiled.transitions[1].approval_hint == {"node_kind": "request_approval"}
    assert compiled.transitions[1].capability is None


def test_compile_workflow_definition_returns_stable_rich_petri_dump() -> None:
    workflow = WorkflowDefinition(
        workflow_id="ingestion-early-steps",
        version="2026-03-06",
        nodes=[
            WorkflowNode(node_id="dispatch-normalize", kind="dispatch_executor", capability="python.transform"),
            WorkflowNode(node_id="complete", kind="complete"),
        ],
        links=[WorkflowLink(source="dispatch-normalize", target="complete")],
    )

    first = compile_workflow_definition(workflow).model_dump(mode="json")
    second = compile_workflow_definition(workflow).model_dump(mode="json")

    assert first == second


def test_compile_workflow_definition_rejects_invalid_link_targets() -> None:
    with pytest.raises(ValueError, match="workflow link target 'missing' is not defined"):
        compile_workflow_definition(
            {
                "workflow_id": "broken-flow",
                "version": "v1",
                "nodes": [{"node_id": "dispatch", "kind": "dispatch_executor", "capability": "python.fetch"}],
                "links": [{"source": "dispatch", "target": "missing"}],
            }
        )
