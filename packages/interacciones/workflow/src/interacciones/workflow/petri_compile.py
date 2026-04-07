"""Richer Petri workflow compilation helpers."""

from __future__ import annotations

from interacciones.schemas import RichPetriWorkflow, WorkflowDefinition

from .compiler import compile_workflow_definition


def compile_workflow_to_rich_petri(workflow: WorkflowDefinition | dict[str, object]) -> RichPetriWorkflow:
    return compile_workflow_definition(workflow)
