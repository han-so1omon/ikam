"""Consolidated workflow source preload."""

from __future__ import annotations

from pathlib import Path

from interacciones.schemas import RichPetriWorkflow

from .compiler import compile_workflow_definition
from .loader import load_workflow_definition


def _default_preseed_root() -> Path:
    return Path(__file__).resolve().parents[5] / "test" / "ikam-perf-report" / "preseed"


def load_compiled_workflows(root: Path | None = None) -> list[RichPetriWorkflow]:
    base_root = root or _default_preseed_root()
    workflows_root = base_root if base_root.name == "workflows" or list(base_root.glob("*.yaml")) else base_root / "workflows"
    compiled: list[RichPetriWorkflow] = []
    for workflow_file in sorted(workflows_root.glob("*.yaml")):
        compiled.append(compile_workflow_definition(load_workflow_definition(workflow_file)))
    return compiled
