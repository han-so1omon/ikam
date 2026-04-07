"""Contract tests for workflow definition schemas."""

from pathlib import Path

import pytest
import yaml
from pydantic import ValidationError

from interacciones.schemas import ResolutionMode, WorkflowDefinition, WorkflowLink, WorkflowNode


ROOT = Path(__file__).resolve().parents[4]
INGESTION_WORKFLOW_FIXTURE = ROOT / "packages/test/ikam-perf-report/preseed/workflows/ingestion-early-parse.yaml"


def test_workflow_definition_uses_explicit_nodes_and_links() -> None:
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
        ],
        links=[
            WorkflowLink(source="dispatch-normalize", target="wait-normalize"),
        ],
    )

    dumped = workflow.model_dump(mode="json")

    assert dumped["workflow_id"] == "ingestion-early-steps"
    assert dumped["nodes"][0]["node_id"] == "dispatch-normalize"
    assert dumped["links"] == [{"source": "dispatch-normalize", "target": "wait-normalize"}]


def test_workflow_node_supports_direct_executor_override() -> None:
    node = WorkflowNode(
        node_id="dispatch-embed",
        kind="dispatch_executor",
        capability="ml.embed",
        resolution_mode=ResolutionMode.DIRECT_EXECUTOR_REF,
        direct_executor_ref="executor://ml-primary",
    )

    dumped = node.model_dump(mode="json")

    assert dumped["resolution_mode"] == "direct_executor_ref"
    assert dumped["direct_executor_ref"] == "executor://ml-primary"


def test_workflow_node_requires_direct_executor_ref_for_direct_resolution() -> None:
    with pytest.raises(ValidationError):
        WorkflowNode(
            node_id="dispatch-embed",
            kind="dispatch_executor",
            capability="ml.embed",
            resolution_mode=ResolutionMode.DIRECT_EXECUTOR_REF,
        )


def test_workflow_definition_requires_nodes() -> None:
    with pytest.raises(ValidationError):
        WorkflowDefinition(workflow_id="wf", version="v1", nodes=[], links=[])


def test_workflow_definition_allows_single_node_without_links() -> None:
    workflow = WorkflowDefinition(
        workflow_id="wf",
        version="v1",
        nodes=[WorkflowNode(node_id="complete", kind="complete")],
        links=[],
    )

    assert workflow.links == []


def test_workflow_definition_rejects_duplicate_node_ids() -> None:
    with pytest.raises(ValidationError):
        WorkflowDefinition(
            workflow_id="wf",
            version="v1",
            nodes=[
                WorkflowNode(node_id="dispatch", kind="dispatch_executor", capability="python.fetch"),
                WorkflowNode(node_id="dispatch", kind="wait_for_result"),
            ],
            links=[WorkflowLink(source="dispatch", target="dispatch")],
        )


def test_workflow_definition_rejects_links_to_unknown_nodes() -> None:
    with pytest.raises(ValidationError):
        WorkflowDefinition(
            workflow_id="wf",
            version="v1",
            nodes=[WorkflowNode(node_id="dispatch", kind="dispatch_executor", capability="python.fetch")],
            links=[WorkflowLink(source="dispatch", target="missing")],
        )


def test_workflow_definition_rejects_duplicate_links() -> None:
    with pytest.raises(ValidationError):
        WorkflowDefinition(
            workflow_id="wf",
            version="v1",
            nodes=[
                WorkflowNode(node_id="dispatch", kind="dispatch_executor", capability="python.fetch"),
                WorkflowNode(node_id="wait", kind="wait_for_result"),
            ],
            links=[
                WorkflowLink(source="dispatch", target="wait"),
                WorkflowLink(source="dispatch", target="wait"),
            ],
        )


def test_workflow_definition_rejects_cyclic_workflow_without_entry_or_exit() -> None:
    with pytest.raises(ValidationError):
        WorkflowDefinition(
            workflow_id="wf",
            version="v1",
            nodes=[WorkflowNode(node_id="loop", kind="route")],
            links=[WorkflowLink(source="loop", target="loop")],
        )


def test_workflow_link_requires_source_and_target() -> None:
    with pytest.raises(ValidationError):
        WorkflowLink(source="node-a", target="")


def test_workflow_node_accepts_extended_orchestration_kinds() -> None:
    accepted_kinds = [
        "route",
        "fail",
        "emit_event",
        "await_event",
        "emit_mcp_call",
        "await_mcp_response",
        "emit_acp_message",
        "await_acp_message",
    ]

    nodes = [WorkflowNode(node_id=f"node-{index}", kind=kind) for index, kind in enumerate(accepted_kinds, start=1)]

    assert [node.kind for node in nodes] == accepted_kinds


def test_workflow_node_rejects_unknown_kind() -> None:
    with pytest.raises(ValidationError):
        WorkflowNode(node_id="node-x", kind="unknown_kind")


def test_dispatch_executor_requires_capability() -> None:
    with pytest.raises(ValidationError):
        WorkflowNode(node_id="dispatch", kind="dispatch_executor")


def test_workflow_node_rejects_direct_executor_ref_without_direct_resolution_mode() -> None:
    with pytest.raises(ValidationError):
        WorkflowNode(
            node_id="dispatch",
            kind="dispatch_executor",
            capability="ml.embed",
            direct_executor_ref="executor://ml-primary",
        )


def test_dispatch_executor_accepts_operator_and_executor_selection_policy() -> None:
    node = WorkflowNode(
        node_id="dispatch",
        kind="dispatch_executor",
        capability="python.chunk_documents",
        operator_selection={"preferred": ["modelado/operators/chunking"]},
        executor_selection={"preferred_capabilities": ["python.chunk_documents"]},
    )

    dumped = node.model_dump(mode="json")

    assert dumped["operator_selection"] == {"preferred": ["modelado/operators/chunking"]}
    assert dumped["executor_selection"] == {"preferred_capabilities": ["python.chunk_documents"]}


def test_non_dispatch_nodes_reject_execution_routing_fields() -> None:
    with pytest.raises(ValidationError):
        WorkflowNode(
            node_id="wait",
            kind="wait_for_result",
            capability="ml.embed",
            resolution_mode=ResolutionMode.DIRECT_EXECUTOR_REF,
            direct_executor_ref="executor://ml-primary",
        )

    with pytest.raises(ValidationError):
        WorkflowNode(
            node_id="wait-policy",
            kind="wait_for_result",
            operator_selection={"preferred": ["modelado/operators/chunking"]},
        )

    with pytest.raises(ValidationError):
        WorkflowNode(
            node_id="wait-executor-policy",
            kind="wait_for_result",
            executor_selection={"preferred_capabilities": ["python.chunk_documents"]},
        )


def test_ingestion_workflow_fixture_declares_validators_on_operator_nodes() -> None:
    payload = yaml.safe_load(INGESTION_WORKFLOW_FIXTURE.read_text(encoding="utf-8"))

    workflow = WorkflowDefinition.model_validate(payload)
    nodes_by_id = {node.node_id: node for node in workflow.nodes}

    assert nodes_by_id["load-documents"].validators
    assert nodes_by_id["parse-chunk"].validators
    assert nodes_by_id["parse-entities-and-relationships"].validators
    assert nodes_by_id["parse-claims"].validators
    assert [validator.name for validator in nodes_by_id["load-documents"].validators] == [
        "input-url",
        "output-document-set",
    ]
    assert [validator.name for validator in nodes_by_id["parse-chunk"].validators] == [
        "input-document-set",
        "output-chunk-extraction-set",
    ]
    assert [validator.name for validator in nodes_by_id["parse-entities-and-relationships"].validators] == [
        "input-chunk-extraction-set",
        "output-entity-relationship-set",
    ]
    assert [validator.name for validator in nodes_by_id["parse-claims"].validators] == [
        "input-entity-relationship-set",
        "output-claim-set",
    ]
    assert all(validator.kind == "type" for validator in nodes_by_id["load-documents"].validators)
    assert all(validator.kind == "type" for validator in nodes_by_id["parse-chunk"].validators)
    assert all(validator.kind == "type" for validator in nodes_by_id["parse-entities-and-relationships"].validators)
    assert all(validator.kind == "type" for validator in nodes_by_id["parse-claims"].validators)
