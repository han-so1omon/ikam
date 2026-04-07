from __future__ import annotations

from dataclasses import replace
from typing import Any, Callable

from interacciones.schemas import ExecutionQueued, ExecutionQueueRequest, OrchestrationTopicNames
from modelado.operators.core import OperatorEnv


class ExecutionQueueBus:
    def __init__(
        self,
        *,
        topics: OrchestrationTopicNames | None = None,
        publish: Callable[[str, dict], None] | None = None,
    ) -> None:
        self._topics = topics or OrchestrationTopicNames()
        self.messages: list[tuple[str, dict]] = []
        self._publish = publish or self._record

    @classmethod
    def from_workflow_bus(cls, workflow_bus: Any) -> "ExecutionQueueBus":
        bus = cls(topics=workflow_bus.topics)
        bus._publish = lambda topic, payload: bus._publish_via_workflow_bus(workflow_bus, topic, payload)
        return bus

    def publish_request(self, request: ExecutionQueueRequest) -> None:
        self._publish(self._topics.execution_requests, request.model_dump(mode="json"))
        self._publish(
            self._topics.workflow_events,
            ExecutionQueued(
                request_id=request.request_id,
                workflow_id=request.workflow_id,
                step_id=request.step_id,
                executor_id=request.executor_id,
                executor_kind=request.executor_kind,
                capability=request.capability,
            ).model_dump(mode="json"),
        )

    def _record(self, topic: str, payload: dict) -> None:
        self.messages.append((topic, payload))

    def _publish_via_workflow_bus(self, workflow_bus: Any, topic: str, payload: dict) -> None:
        if topic == workflow_bus.topics.execution_requests:
            published = workflow_bus.publish_execution_request(payload)
        elif topic == workflow_bus.topics.workflow_events:
            published = workflow_bus.publish_workflow_event(payload)
        else:
            raise ValueError(f"unsupported workflow bus topic: {topic}")
        self.messages.append((published.topic, published.payload))


def attach_execution_queue_bus(env: OperatorEnv, *, workflow_bus: Any) -> OperatorEnv:
    slots = dict(env.slots)
    slots["execution_queue_bus"] = ExecutionQueueBus.from_workflow_bus(workflow_bus)
    return replace(env, slots=slots)
