from __future__ import annotations

from interacciones.graph.workflow.bus import PublishedMessage, WorkflowBus
from interacciones.graph.workflow.resolver import WorkflowExecutorResolver
from interacciones.schemas.execution import ExecutionRequest
from interacciones.schemas.executors import ExecutorDeclaration


class WorkflowExecutionDispatcher:
    def __init__(self, declarations: list[ExecutorDeclaration], *, bus: WorkflowBus | None = None) -> None:
        self._resolver = WorkflowExecutorResolver(declarations)
        self._bus = bus or WorkflowBus()

    def dispatch(self, request: ExecutionRequest) -> PublishedMessage:
        executor = self._resolver.resolve(request)
        return self._bus.publish_execution_request(
            {
                "request_id": request.request_id,
                "workflow_id": request.workflow_id,
                "step_id": request.step_id,
                "executor_id": executor.executor_id,
                "executor_kind": executor.executor_kind,
                "capability": request.capability,
                "policy": request.policy,
                "constraints": request.constraints,
                "payload": request.payload,
                "transport": executor.transport,
            }
        )
