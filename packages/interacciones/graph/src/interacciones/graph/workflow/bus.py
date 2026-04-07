from __future__ import annotations

from copy import deepcopy
from dataclasses import dataclass
from typing import Any

from interacciones.schemas.topics import OrchestrationTopicNames


@dataclass(frozen=True)
class PublishedMessage:
    topic: str
    payload: dict[str, Any]


class WorkflowBus:
    def __init__(self, *, topics: OrchestrationTopicNames | None = None) -> None:
        self.topics = topics or OrchestrationTopicNames()

    def topic_for(self, channel: str) -> str:
        return self.topics.topic_for(channel)

    def publish_execution_request(self, payload: dict[str, Any]) -> PublishedMessage:
        return self._publish("execution_requests", payload)

    def publish_execution_progress(self, payload: dict[str, Any]) -> PublishedMessage:
        return self._publish("execution_progress", payload)

    def publish_execution_result(self, payload: dict[str, Any]) -> PublishedMessage:
        return self._publish("execution_results", payload)

    def publish_workflow_event(self, payload: dict[str, Any]) -> PublishedMessage:
        return self._publish("workflow_events", payload)

    def publish_approval_event(self, payload: dict[str, Any]) -> PublishedMessage:
        return self._publish("approval_events", payload)

    def publish_mcp_event(self, payload: dict[str, Any]) -> PublishedMessage:
        return self._publish("mcp_events", payload)

    def publish_acp_event(self, payload: dict[str, Any]) -> PublishedMessage:
        return self._publish("acp_events", payload)

    def _publish(self, channel: str, payload: dict[str, Any]) -> PublishedMessage:
        return PublishedMessage(topic=self.topic_for(channel), payload=deepcopy(payload))
