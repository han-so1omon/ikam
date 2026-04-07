"""Tests for workflow template loading and validation."""

from __future__ import annotations

import json

from interacciones.schemas import ResolutionMode
from interacciones.workflow.loader import load_workflow_definition


def test_load_workflow_definition_from_yaml_preserves_explicit_nodes_and_links(tmp_path) -> None:
    workflow_path = tmp_path / "workflow.yaml"
    workflow_path.write_text(
        """
workflow_id: ingestion-early-steps
version: "2026-03-06"
nodes:
  - node_id: dispatch-normalize
    kind: dispatch_executor
    capability: python.transform
    policy:
      cost_tier: standard
  - node_id: wait-normalize
    kind: wait_for_result
links:
  - source: dispatch-normalize
    target: wait-normalize
""".strip(),
        encoding="utf-8",
    )

    workflow = load_workflow_definition(workflow_path)

    assert workflow.workflow_id == "ingestion-early-steps"
    assert [node.node_id for node in workflow.nodes] == ["dispatch-normalize", "wait-normalize"]
    assert workflow.links[0].source == "dispatch-normalize"
    assert workflow.links[0].target == "wait-normalize"


def test_load_workflow_definition_from_json_supports_direct_executor_override(tmp_path) -> None:
    workflow_path = tmp_path / "workflow.json"
    workflow_path.write_text(
        json.dumps(
            {
                "workflow_id": "embed-flow",
                "version": "v1",
                "nodes": [
                    {
                        "node_id": "dispatch-embed",
                        "kind": "dispatch_executor",
                        "capability": "ml.embed",
                        "resolution_mode": "direct_executor_ref",
                        "direct_executor_ref": "executor://ml-primary",
                    },
                    {"node_id": "wait-embed", "kind": "wait_for_result"},
                ],
                "links": [{"source": "dispatch-embed", "target": "wait-embed"}],
            }
        ),
        encoding="utf-8",
    )

    workflow = load_workflow_definition(workflow_path)

    assert workflow.nodes[0].resolution_mode is ResolutionMode.DIRECT_EXECUTOR_REF
    assert workflow.nodes[0].direct_executor_ref == "executor://ml-primary"
