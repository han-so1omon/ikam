from __future__ import annotations

import json
from typing import Any, Callable

from interacciones.schemas import OrchestrationTopicNames


class RedpandaExecutorBroker:
    def __init__(
        self,
        *,
        topics: OrchestrationTopicNames | None = None,
        consumer_factory: Callable[[str], Any] | None = None,
        producer_factory: Callable[[], Any] | None = None,
        consumer_group_id: str,
    ) -> None:
        self._topics = topics or OrchestrationTopicNames()
        if consumer_factory is None or producer_factory is None:
            from interacciones.kafka import build_consumer, build_producer

            consumer_factory = consumer_factory or build_consumer
            producer_factory = producer_factory or build_producer
        self._consumer = consumer_factory(consumer_group_id)
        self._producer = producer_factory()
        self._subscribed_topics: set[str] = set()

    def poll(self, topic: str) -> tuple[str, dict[str, Any]] | None:
        if topic not in self._subscribed_topics:
            self._consumer.subscribe([topic])
            self._subscribed_topics.add(topic)
        message = self._consumer.poll(1.0)
        if message is None:
            return None
        error = message.error()
        if error is not None:
            raise RuntimeError(str(error))
        payload = message.value()
        if payload is None:
            return None
        return message.topic(), json.loads(payload.decode("utf-8"))

    def publish(self, topic: str, payload: dict[str, Any]) -> None:
        self._producer.produce(topic, json.dumps(payload, sort_keys=True).encode("utf-8"))
        self._producer.flush(1.0)

    def commit(self, topic: str) -> None:
        if topic not in self._subscribed_topics:
            return
        self._consumer.commit(asynchronous=False)
