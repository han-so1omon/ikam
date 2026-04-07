"""Validation helpers for workflow templates."""

from __future__ import annotations

from collections.abc import Mapping
from typing import Any

from interacciones.schemas import WorkflowDefinition


def validate_workflow_definition(value: WorkflowDefinition | Mapping[str, Any]) -> WorkflowDefinition:
    if isinstance(value, WorkflowDefinition):
        return WorkflowDefinition.model_validate(value.model_dump(mode="python"))
    return WorkflowDefinition.model_validate(dict(value))
