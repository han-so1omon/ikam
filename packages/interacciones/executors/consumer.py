from __future__ import annotations

from interacciones.schemas import OrchestrationTopicNames


class SharedQueueExecutorConsumer:
    def __init__(self, executor: object, *, topics: OrchestrationTopicNames | None = None) -> None:
        self._executor = executor
        self._topics = topics or OrchestrationTopicNames()

    def consume(self, *, topic: str, payload: dict) -> None:
        if topic != self._topics.execution_requests:
            raise ValueError(f"unsupported executor topic: {topic}")
        self._executor.consume(topic=topic, payload=payload)
