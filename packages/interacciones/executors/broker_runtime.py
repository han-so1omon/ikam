from __future__ import annotations

from typing import Any, Callable


class BrokerBackedExecutorRuntime:
    def __init__(self, *, broker: Any, consumer: Any, publisher: Any, topics: Any) -> None:
        self._broker = broker
        self._consumer = consumer
        self._publisher = publisher
        self._topics = topics

    def run_once(self) -> bool:
        message = self._broker.poll(self._topics.execution_requests)
        if message is None:
            return False
        topic, payload = message
        before = len(self._publisher.messages)
        self._consumer.consume(topic=topic, payload=payload)
        for item in self._publisher.messages[before:]:
            self._broker.publish(str(item["topic"]), dict(item["payload"]))
        self._broker.commit(topic)
        return True

    def run_forever(
        self,
        *,
        max_iterations: int | None = None,
        should_continue: Callable[[int], bool] | None = None,
    ) -> int:
        iteration = 0
        while True:
            if max_iterations is not None and iteration >= max_iterations:
                return iteration
            if should_continue is not None and not should_continue(iteration):
                return iteration
            self.run_once()
            iteration += 1
