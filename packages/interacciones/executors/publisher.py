from __future__ import annotations

from interacciones.schemas import ExecutionCompleted, ExecutionFailed, ExecutionProgress, OrchestrationTopicNames


class InMemoryExecutionEventPublisher:
    def __init__(self) -> None:
        self.published: list[dict[str, object]] = []

    def publish_progress(self, event: ExecutionProgress) -> None:
        self.published.append({"kind": "progress", "event": event.model_dump(mode="json")})

    def publish_completed(self, event: ExecutionCompleted) -> None:
        self.published.append({"kind": "completed", "event": event.model_dump(mode="json")})

    def publish_failed(self, event: ExecutionFailed) -> None:
        self.published.append({"kind": "failed", "event": event.model_dump(mode="json")})


class TopicExecutionEventPublisher:
    def __init__(self, *, topics: OrchestrationTopicNames | None = None) -> None:
        self._topics = topics or OrchestrationTopicNames()
        self.messages: list[dict[str, object]] = []

    def publish_progress(self, event: ExecutionProgress) -> None:
        self.messages.append(
            {
                "kind": "progress",
                "topic": self._topics.execution_progress,
                "payload": event.model_dump(mode="json"),
            }
        )

    def publish_completed(self, event: ExecutionCompleted) -> None:
        self.messages.append(
            {
                "kind": "completed",
                "topic": self._topics.execution_results,
                "payload": event.model_dump(mode="json"),
            }
        )

    def publish_failed(self, event: ExecutionFailed) -> None:
        self.messages.append(
            {
                "kind": "failed",
                "topic": self._topics.execution_results,
                "payload": event.model_dump(mode="json"),
            }
        )
