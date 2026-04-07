from __future__ import annotations

from interacciones.graph.workflow.event_handlers import WorkflowExecutionEventHandlers
from interacciones.schemas.topics import OrchestrationTopicNames


class WorkflowTopicConsumer:
    def __init__(self, handlers: WorkflowExecutionEventHandlers, *, topics: OrchestrationTopicNames | None = None) -> None:
        self._handlers = handlers
        self._topics = topics or OrchestrationTopicNames()

    def consume(self, topic: str, payload: dict) -> None:
        if self._topics.matches("workflow_events", topic):
            if payload.get("status") == "queued":
                self._handlers.handle_queued(payload)
                return
            raise ValueError(f"unsupported workflow event payload: {payload}")
        if self._topics.matches("execution_progress", topic):
            self._handlers.handle_progress(payload)
            return
        if self._topics.matches("execution_results", topic):
            if payload.get("error_code"):
                self._handlers.handle_failed(payload)
                return
            self._handlers.handle_completed(payload)
            return
        if self._topics.matches("approval_events", topic):
            if payload.get("resolved_by"):
                self._handlers.handle_approval_resolved(payload)
                return
            self._handlers.handle_approval_requested(payload)
            return
        raise ValueError(f"unsupported workflow topic: {topic}")
