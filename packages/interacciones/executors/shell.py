from __future__ import annotations

from typing import Any, Callable

from interacciones.schemas import OrchestrationTopicNames

from broker_runtime import BrokerBackedExecutorRuntime
from consumer import SharedQueueExecutorConsumer
from ml_executor import MlExecutorService
from publisher import TopicExecutionEventPublisher
from python_executor import PythonExecutorService
from redpanda_broker import RedpandaExecutorBroker


def build_python_executor_runtime(
    *,
    executor_id: str,
    handlers: dict[str, Callable[[dict[str, Any]], dict[str, Any]]],
    consumer_group_id: str,
    topics: OrchestrationTopicNames | None = None,
    consumer_factory: Callable[[str], Any] | None = None,
    producer_factory: Callable[[], Any] | None = None,
) -> BrokerBackedExecutorRuntime:
    return _build_runtime(
        executor=PythonExecutorService,
        executor_id=executor_id,
        handlers=handlers,
        consumer_group_id=consumer_group_id,
        topics=topics,
        consumer_factory=consumer_factory,
        producer_factory=producer_factory,
    )


def build_ml_executor_runtime(
    *,
    executor_id: str,
    handlers: dict[str, Callable[[dict[str, Any]], dict[str, Any]]],
    consumer_group_id: str,
    topics: OrchestrationTopicNames | None = None,
    consumer_factory: Callable[[str], Any] | None = None,
    producer_factory: Callable[[], Any] | None = None,
) -> BrokerBackedExecutorRuntime:
    return _build_runtime(
        executor=MlExecutorService,
        executor_id=executor_id,
        handlers=handlers,
        consumer_group_id=consumer_group_id,
        topics=topics,
        consumer_factory=consumer_factory,
        producer_factory=producer_factory,
    )


def _build_runtime(
    *,
    executor: Any,
    executor_id: str,
    handlers: dict[str, Callable[[dict[str, Any]], dict[str, Any]]],
    consumer_group_id: str,
    topics: OrchestrationTopicNames | None,
    consumer_factory: Callable[[str], Any] | None,
    producer_factory: Callable[[], Any] | None,
) -> BrokerBackedExecutorRuntime:
    resolved_topics = topics or OrchestrationTopicNames()
    publisher = TopicExecutionEventPublisher(topics=resolved_topics)
    service = executor(executor_id=executor_id, publisher=publisher, handlers=handlers)
    consumer = SharedQueueExecutorConsumer(service, topics=resolved_topics)
    broker = RedpandaExecutorBroker(
        topics=resolved_topics,
        consumer_factory=consumer_factory,
        producer_factory=producer_factory,
        consumer_group_id=consumer_group_id,
    )
    return BrokerBackedExecutorRuntime(
        broker=broker,
        consumer=consumer,
        publisher=publisher,
        topics=resolved_topics,
    )
