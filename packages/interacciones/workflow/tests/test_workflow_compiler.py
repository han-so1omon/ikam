"""Tests for workflow frontend compilation."""

from __future__ import annotations

import pytest

from interacciones.schemas import ResolutionMode, RichPetriWorkflow, WorkflowDefinition, WorkflowLink, WorkflowNode
from interacciones.workflow.compiler import compile_workflow_definition


def test_compile_workflow_definition_builds_rich_petri_workflow() -> None:
    workflow = WorkflowDefinition(
        workflow_id="ingestion-early-steps",
        version="2026-03-06",
        nodes=[
            WorkflowNode(
                node_id="dispatch-normalize",
                kind="dispatch_executor",
                capability="python.transform",
                policy={"cost_tier": "standard"},
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
    assert compiled.version == "2026-03-06"
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
    assert compiled.transitions[0].approval_hint == {}
    assert compiled.source_workflow_storage.mode.value == "default_on"
    assert compiled.source_workflow_definition.model_dump(mode="json") == workflow.model_dump(mode="json")


def test_compile_workflow_definition_preserves_resolution_mode_and_policy_input() -> None:
    workflow = WorkflowDefinition(
        workflow_id="embed-flow",
        version="v1",
        nodes=[
            WorkflowNode(
                node_id="dispatch-embed",
                kind="dispatch_executor",
                capability="ml.embed",
                policy={"routing": {"tiers": ["primary"]}},
                resolution_mode=ResolutionMode.DIRECT_EXECUTOR_REF,
                direct_executor_ref="executor://ml-primary",
            ),
            WorkflowNode(node_id="wait-embed", kind="wait_for_result"),
        ],
        links=[WorkflowLink(source="dispatch-embed", target="wait-embed")],
    )

    compiled = compile_workflow_definition(workflow)

    assert compiled.transitions[0].resolution_mode is ResolutionMode.DIRECT_EXECUTOR_REF
    assert compiled.transitions[0].direct_executor_ref == "executor://ml-primary"
    assert compiled.transitions[0].policy == {"routing": {"tiers": ["primary"]}}


def test_compile_workflow_definition_preserves_operator_and_executor_selection_policy() -> None:
    workflow = WorkflowDefinition(
        workflow_id="chunk-flow",
        version="v1",
        nodes=[
            WorkflowNode(
                node_id="parse-chunk",
                kind="dispatch_executor",
                capability="python.chunk_documents",
                policy={"cost_tier": "standard"},
                operator_selection={"preferred": ["modelado/operators/chunking"]},
                executor_selection={"preferred_capabilities": ["python.chunk_documents"]},
            ),
            WorkflowNode(node_id="wait", kind="wait_for_result"),
        ],
        links=[WorkflowLink(source="parse-chunk", target="wait")],
    )

    compiled = compile_workflow_definition(workflow)

    assert compiled.transitions[0].policy == {
        "cost_tier": "standard",
        "operator_selection": {"preferred": ["modelado/operators/chunking"]},
        "executor_selection": {"preferred_capabilities": ["python.chunk_documents"]},
    }


def test_compile_workflow_definition_preserves_authored_policy_keys_when_selection_fields_overlap() -> None:
    workflow = WorkflowDefinition(
        workflow_id="chunk-flow-authored-policy",
        version="v1",
        nodes=[
            WorkflowNode(
                node_id="parse-chunk",
                kind="dispatch_executor",
                capability="python.chunk_documents",
                policy={
                    "operator_selection": {"preferred": ["modelado/operators/authored"]},
                    "executor_selection": {"preferred_capabilities": ["python.authored"]},
                },
                operator_selection={"preferred": ["modelado/operators/chunking"]},
                executor_selection={"preferred_capabilities": ["python.chunk_documents"]},
            ),
        ],
    )

    compiled = compile_workflow_definition(workflow)

    assert compiled.transitions[0].policy == {
        "operator_selection": {"preferred": ["modelado/operators/authored"]},
        "executor_selection": {"preferred_capabilities": ["python.authored"]},
    }


def test_compile_workflow_definition_preserves_transition_validators() -> None:
    workflow = WorkflowDefinition(
        workflow_id="validated-flow",
        version="v1",
        nodes=[
            WorkflowNode(
                node_id="load-documents",
                kind="dispatch_executor",
                capability="python.load_documents",
                validators=[
                    {
                        "name": "input-url",
                        "direction": "input",
                        "kind": "type",
                        "selector": "input.url",
                        "target": "value",
                        "config": {
                            "schema": {
                                "type": "object",
                                "title": "url",
                                "required": ["kind", "location"],
                            }
                        },
                    }
                ],
            ),
            WorkflowNode(node_id="parse-chunk", kind="dispatch_executor", capability="python.chunk_documents"),
        ],
        links=[WorkflowLink(source="load-documents", target="parse-chunk")],
    )

    compiled = compile_workflow_definition(workflow)

    assert [validator.model_dump(mode="json") for validator in compiled.transitions[0].validators] == [
        {
            "name": "input-url",
            "direction": "input",
            "kind": "type",
            "selector": "input.url",
            "target": "value",
            "config": {
                "schema": {
                    "type": "object",
                    "title": "url",
                    "required": ["kind", "location"],
                }
            },
        }
    ]


def test_compile_workflow_definition_preserves_downstream_semantic_transition_validators() -> None:
    workflow = WorkflowDefinition(
        workflow_id="validated-flow-downstream",
        version="v1",
        nodes=[
            WorkflowNode(
                node_id="parse-entities-and-relationships",
                kind="dispatch_executor",
                capability="ml.extract_entities_relationships",
                validators=[
                    {
                        "name": "input-chunk-extraction-set",
                        "direction": "input",
                        "kind": "type",
                        "selector": "input.chunk_extraction_set",
                        "target": "value",
                        "config": {
                            "schema": {
                                "type": "object",
                                "title": "chunk_extraction_set",
                                "required": ["kind", "source_subgraph_ref", "subgraph_ref", "extraction_refs"],
                            }
                        },
                    },
                    {
                        "name": "output-entity-relationship-set",
                        "direction": "output",
                        "kind": "type",
                        "selector": "output.entity_relationship_set",
                        "target": "value",
                        "config": {
                            "schema": {
                                "type": "object",
                                "title": "entity_relationship_set",
                                "required": ["kind", "source_subgraph_ref", "subgraph_ref", "entity_relationship_refs"],
                            }
                        },
                    },
                ],
            ),
            WorkflowNode(
                node_id="parse-claims",
                kind="dispatch_executor",
                capability="ml.extract_claims",
                validators=[
                    {
                        "name": "input-entity-relationship-set",
                        "direction": "input",
                        "kind": "type",
                        "selector": "input.entity_relationship_set",
                        "target": "value",
                        "config": {
                            "schema": {
                                "type": "object",
                                "title": "entity_relationship_set",
                                "required": ["kind", "source_subgraph_ref", "subgraph_ref", "entity_relationship_refs"],
                            }
                        },
                    },
                    {
                        "name": "output-claim-set",
                        "direction": "output",
                        "kind": "type",
                        "selector": "output.claim_set",
                        "target": "value",
                        "config": {
                            "schema": {
                                "type": "object",
                                "title": "claim_set",
                                "required": ["kind", "source_subgraph_ref", "subgraph_ref", "claim_refs"],
                            }
                        },
                    },
                ],
            ),
        ],
        links=[WorkflowLink(source="parse-entities-and-relationships", target="parse-claims")],
    )

    compiled = compile_workflow_definition(workflow)

    assert [validator.name for validator in compiled.transitions[0].validators] == [
        "input-chunk-extraction-set",
        "output-entity-relationship-set",
    ]
    assert [validator.name for validator in compiled.transitions[1].validators] == [
        "input-entity-relationship-set",
        "output-claim-set",
    ]


def test_compile_workflow_definition_maps_structural_hints_from_node_kinds() -> None:
    workflow = WorkflowDefinition(
        workflow_id="control-flow",
        version="v1",
        nodes=[
            WorkflowNode(node_id="approve", kind="request_approval"),
            WorkflowNode(node_id="checkpoint", kind="checkpoint"),
        ],
        links=[WorkflowLink(source="approve", target="checkpoint")],
    )

    compiled = compile_workflow_definition(workflow)

    assert compiled.transitions[0].approval_hint == {"node_kind": "request_approval"}
    assert compiled.transitions[1].checkpoint_hint == {"node_kind": "checkpoint"}
    assert compiled.transitions[0].capability is None
    assert compiled.transitions[1].capability is None


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


def test_compile_workflow_definition_rejects_cycles_without_entry_or_exit() -> None:
    with pytest.raises(ValueError, match="workflow must define at least one entry node"):
        compile_workflow_definition(
            {
                "workflow_id": "cycle-flow",
                "version": "v1",
                "nodes": [{"node_id": "loop", "kind": "route"}],
                "links": [{"source": "loop", "target": "loop"}],
            }
        )


def test_compile_workflow_definition_revalidates_constructed_models() -> None:
    with pytest.raises(ValueError, match="workflow link target 'missing' is not defined"):
        compile_workflow_definition(
            WorkflowDefinition.model_construct(
                workflow_id="broken-flow",
                version="v1",
                nodes=[WorkflowNode.model_construct(node_id="dispatch", kind="dispatch_executor", capability="python.fetch")],
                links=[WorkflowLink(source="dispatch", target="missing")],
            )
        )


def test_compile_workflow_definition_rejects_unknown_node_kinds() -> None:
    with pytest.raises(ValueError, match="workflow node kind 'unknown_kind' is not supported"):
        compile_workflow_definition(
            WorkflowDefinition.model_construct(
                workflow_id="future-flow",
                version="v1",
                nodes=[WorkflowNode.model_construct(node_id="emit", kind="unknown_kind")],
                links=[WorkflowLink(source="emit", target="emit")],
            )
        )


def test_compile_workflow_definition_supports_extended_control_node_kinds() -> None:
    workflow = WorkflowDefinition(
        workflow_id="control-surface",
        version="v1",
        nodes=[
            WorkflowNode(node_id="route", kind="route"),
            WorkflowNode(node_id="emit-event", kind="emit_event"),
            WorkflowNode(node_id="await-event", kind="await_event"),
            WorkflowNode(node_id="emit-mcp", kind="emit_mcp_call"),
            WorkflowNode(node_id="await-mcp", kind="await_mcp_response"),
            WorkflowNode(node_id="emit-acp", kind="emit_acp_message"),
            WorkflowNode(node_id="await-acp", kind="await_acp_message"),
            WorkflowNode(node_id="fail", kind="fail"),
        ],
        links=[
            WorkflowLink(source="route", target="emit-event"),
            WorkflowLink(source="route", target="fail"),
            WorkflowLink(source="emit-event", target="await-event"),
            WorkflowLink(source="await-event", target="emit-mcp"),
            WorkflowLink(source="emit-mcp", target="await-mcp"),
            WorkflowLink(source="await-mcp", target="emit-acp"),
            WorkflowLink(source="emit-acp", target="await-acp"),
        ],
    )

    compiled = compile_workflow_definition(workflow)

    transition_hints = {transition.transition_id: transition.checkpoint_hint for transition in compiled.transitions}

    assert transition_hints["transition:route"] == {"node_kind": "route"}
    assert transition_hints["transition:emit-event"] == {"node_kind": "emit_event"}
    assert transition_hints["transition:await-event"] == {"node_kind": "await_event"}
    assert transition_hints["transition:emit-mcp"] == {"node_kind": "emit_mcp_call"}
    assert transition_hints["transition:await-mcp"] == {"node_kind": "await_mcp_response"}
    assert transition_hints["transition:emit-acp"] == {"node_kind": "emit_acp_message"}
    assert transition_hints["transition:await-acp"] == {"node_kind": "await_acp_message"}
    assert transition_hints["transition:fail"] == {"node_kind": "fail"}


def test_compile_workflow_definition_preserves_explicit_branching_links() -> None:
    workflow = WorkflowDefinition(
        workflow_id="branching-flow",
        version="v1",
        nodes=[
            WorkflowNode(node_id="route", kind="route"),
            WorkflowNode(node_id="fast-path", kind="emit_event"),
            WorkflowNode(node_id="slow-path", kind="request_approval"),
        ],
        links=[
            WorkflowLink(source="route", target="fast-path"),
            WorkflowLink(source="route", target="slow-path"),
        ],
    )

    compiled = compile_workflow_definition(workflow)

    assert [place.place_id for place in compiled.places] == [
        "place:start:route",
        "place:link:route:fast-path",
        "place:link:route:slow-path",
        "place:end:fast-path",
        "place:end:slow-path",
    ]
