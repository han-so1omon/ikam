from __future__ import annotations

from interacciones.graph.workflow.orchestrator import WorkflowOrchestrator
from interacciones.schemas.execution import ApprovalRequested, ApprovalResolved, ExecutionCompleted, ExecutionFailed, ExecutionProgress, ExecutionQueued


class WorkflowExecutionEventHandlers:
    def __init__(self, orchestrator: WorkflowOrchestrator) -> None:
        self._orchestrator = orchestrator

    def handle_progress(self, payload: dict) -> None:
        self._orchestrator.handle_execution_event(ExecutionProgress.model_validate(payload))

    def handle_queued(self, payload: dict) -> None:
        self._orchestrator.handle_execution_event(ExecutionQueued.model_validate(payload))

    def handle_completed(self, payload: dict) -> None:
        self._orchestrator.handle_execution_event(ExecutionCompleted.model_validate(payload))

    def handle_failed(self, payload: dict) -> None:
        self._orchestrator.handle_execution_event(ExecutionFailed.model_validate(payload))

    def handle_approval_requested(self, payload: dict) -> None:
        self._orchestrator.handle_approval_requested(ApprovalRequested.model_validate(payload))

    def handle_approval_resolved(self, payload: dict) -> None:
        self._orchestrator.handle_approval_resolved(ApprovalResolved.model_validate(payload))
