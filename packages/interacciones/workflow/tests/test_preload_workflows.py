"""Tests for consolidated workflow preload."""

from __future__ import annotations

from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "packages/interacciones/workflow/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))

from interacciones.workflow.preload_workflows import load_compiled_workflows


def test_load_compiled_workflows_reads_consolidated_workflow_sources() -> None:
    workflows = load_compiled_workflows()

    assert [workflow.workflow_id for workflow in workflows] == ["ingestion-early-parse"]
    assert [transition.transition_id for transition in workflows[0].transitions] == [
        "transition:init-initialize",
        "transition:load-documents",
        "transition:parse-chunk",
        "transition:parse-entities-and-relationships",
        "transition:parse-claims",
        "transition:complete",
    ]


def test_load_compiled_workflows_preserves_publish_targets() -> None:
    workflows = load_compiled_workflows()

    assert workflows[0].publish[0].registry == "petri_net_runnables"
    assert workflows[0].publish[0].key == "ingestion-early-parse"
