"""Workflow template file loaders."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import yaml

from interacciones.schemas import WorkflowDefinition

from .validator import validate_workflow_definition


def load_workflow_definition(path: str | Path) -> WorkflowDefinition:
    template_path = Path(path)
    raw = template_path.read_text(encoding="utf-8")
    data = _load_template_data(template_path, raw)
    return validate_workflow_definition(data)


def _load_template_data(path: Path, raw: str) -> dict[str, Any]:
    suffix = path.suffix.lower()
    if suffix == ".json":
        data = json.loads(raw)
    elif suffix in {".yaml", ".yml"}:
        data = yaml.safe_load(raw)
    else:
        raise ValueError(f"unsupported workflow template format: {path.suffix}")
    if not isinstance(data, dict):
        raise ValueError("workflow template must decode to an object")
    return data
