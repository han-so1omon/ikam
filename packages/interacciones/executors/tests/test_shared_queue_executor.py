from __future__ import annotations

import os
from pathlib import Path
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "packages/interacciones/executors"))
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))
EXECUTOR_PYTHONPATH = os.pathsep.join(
    [
        str(ROOT / "packages/interacciones/executors"),
        str(ROOT / "packages/interacciones/schemas/src"),
    ]
)


def _dev_topics():
    from interacciones.schemas import OrchestrationTopicNames

    return OrchestrationTopicNames(
        execution_requests="dev.execution.requests",
        execution_progress="dev.execution.progress",
        execution_results="dev.execution.results",
        workflow_events="dev.workflow.events",
        approval_events="dev.approval.events",
        mcp_events="dev.mcp.events",
        acp_events="dev.acp.events",
    )


def _imports():
    from ml_executor import MlExecutorService
    from publisher import InMemoryExecutionEventPublisher
    from python_executor import PythonExecutorService

    return MlExecutorService, InMemoryExecutionEventPublisher, PythonExecutorService


def test_python_executor_consumes_shared_queue_request_and_publishes_shared_events() -> None:
    _, InMemoryExecutionEventPublisher, PythonExecutorService = _imports()
    publisher = InMemoryExecutionEventPublisher()
    executor = PythonExecutorService(
        executor_id="executor://python-primary",
        publisher=publisher,
        handlers={"python.parse_artifacts": lambda payload: {"documents": [payload["input"].upper()]}},
    )

    executor.consume(
        topic="execution.requests",
        payload={
            "request_id": "req-1",
            "workflow_id": "wf-1",
            "step_id": "dispatch-parse",
            "executor_id": "executor://python-primary",
            "executor_kind": "python-executor",
            "capability": "python.parse_artifacts",
            "policy": {"cost_tier": "standard"},
            "constraints": {},
            "payload": {"input": "hola"},
            "transport": {"kind": "redpanda", "request_topic": "execution.requests"},
        },
    )

    assert [item["kind"] for item in publisher.published] == ["progress", "completed"]
    assert publisher.published[0]["event"] == {
        "request_id": "req-1",
        "workflow_id": "wf-1",
        "step_id": "dispatch-parse",
        "executor_id": "executor://python-primary",
        "status": "running",
        "progress": 0.0,
        "message": "started",
        "stdout_lines": [],
        "stderr_lines": [],
        "details": {},
    }
    assert publisher.published[1]["event"] == {
        "request_id": "req-1",
        "workflow_id": "wf-1",
        "step_id": "dispatch-parse",
        "executor_id": "executor://python-primary",
        "result": {"documents": ["HOLA"]},
        "artifacts": [],
        "stdout_lines": [],
        "stderr_lines": [],
    }


def test_ml_executor_consumes_same_queue_request_family_and_publishes_shared_events() -> None:
    MlExecutorService, InMemoryExecutionEventPublisher, _ = _imports()
    publisher = InMemoryExecutionEventPublisher()
    executor = MlExecutorService(
        executor_id="executor://ml-primary",
        publisher=publisher,
        handlers={"ml.embed": lambda payload: {"embedding": [len(payload["input"]), 1.0]}},
    )

    executor.consume(
        topic="execution.requests",
        payload={
            "request_id": "req-2",
            "workflow_id": "wf-2",
            "step_id": "dispatch-embed",
            "executor_id": "executor://ml-primary",
            "executor_kind": "ml-executor",
            "capability": "ml.embed",
            "policy": {"latency_tier": "interactive"},
            "constraints": {},
            "payload": {"input": "hola"},
            "transport": {"kind": "redpanda", "request_topic": "execution.requests"},
        },
    )

    assert [item["kind"] for item in publisher.published] == ["progress", "completed"]
    assert publisher.published[1]["event"] == {
        "request_id": "req-2",
        "workflow_id": "wf-2",
        "step_id": "dispatch-embed",
        "executor_id": "executor://ml-primary",
        "result": {"embedding": [4, 1.0]},
        "artifacts": [],
        "stdout_lines": [],
        "stderr_lines": [],
    }


def test_shared_queue_executor_publishes_failure_for_unsupported_capability() -> None:
    _, InMemoryExecutionEventPublisher, PythonExecutorService = _imports()
    publisher = InMemoryExecutionEventPublisher()
    executor = PythonExecutorService(
        executor_id="executor://python-primary",
        publisher=publisher,
        handlers={"python.parse_artifacts": lambda payload: payload},
    )

    executor.consume(
        topic="execution.requests",
        payload={
            "request_id": "req-3",
            "workflow_id": "wf-3",
            "step_id": "dispatch-embed",
            "executor_id": "executor://python-primary",
            "executor_kind": "python-executor",
            "capability": "ml.embed",
            "policy": {},
            "constraints": {},
            "payload": {"input": "hola"},
            "transport": {"kind": "redpanda", "request_topic": "execution.requests"},
        },
    )

    assert publisher.published == [
        {
            "kind": "failed",
            "event": {
                "request_id": "req-3",
                "workflow_id": "wf-3",
                "step_id": "dispatch-embed",
                "executor_id": "executor://python-primary",
                "error_code": "unsupported_capability",
                "error_message": "unsupported capability: ml.embed",
                "retryable": False,
                "stdout_lines": [],
                "stderr_lines": [],
                "details": {},
            },
        }
    ]


def test_shared_queue_executor_publishes_progressive_stdout_chunks() -> None:
    _, InMemoryExecutionEventPublisher, PythonExecutorService = _imports()
    publisher = InMemoryExecutionEventPublisher()

    def handler(payload: dict[str, object]) -> dict[str, object]:
        print("chunk 1")
        print("chunk 2")
        return {"ok": True, "payload": payload}

    executor = PythonExecutorService(
        executor_id="executor://python-primary",
        publisher=publisher,
        handlers={"python.parse_artifacts": handler},
    )

    executor.consume(
        topic="execution.requests",
        payload={
            "request_id": "req-progressive-1",
            "workflow_id": "wf-progressive-1",
            "step_id": "dispatch-parse",
            "executor_id": "executor://python-primary",
            "executor_kind": "python-executor",
            "capability": "python.parse_artifacts",
            "policy": {},
            "constraints": {},
            "payload": {"input": "hola"},
            "transport": {"kind": "redpanda", "request_topic": "execution.requests"},
        },
    )

    progress_events: list[dict[str, object]] = []
    completed_events: list[dict[str, object]] = []
    for item in publisher.published:
        event = item.get("event")
        if not isinstance(event, dict):
            continue
        if item["kind"] == "progress":
            progress_events.append(event)
        if item["kind"] == "completed":
            completed_events.append(event)

    assert len(progress_events) >= 3
    assert progress_events[0]["step_id"] == "dispatch-parse"
    assert any(event["stdout_lines"] == ["chunk 1"] for event in progress_events)
    assert any(event["stdout_lines"] == ["chunk 2"] for event in progress_events)
    assert all(event["stderr_lines"] == [] for event in progress_events)
    assert completed_events == [
        {
            "request_id": "req-progressive-1",
            "workflow_id": "wf-progressive-1",
            "step_id": "dispatch-parse",
            "executor_id": "executor://python-primary",
            "result": {"ok": True, "payload": {"input": "hola"}},
            "artifacts": [],
            "stdout_lines": [],
            "stderr_lines": [],
        }
    ]


def test_shared_queue_executor_ignores_request_addressed_to_different_executor() -> None:
    _, InMemoryExecutionEventPublisher, PythonExecutorService = _imports()
    publisher = InMemoryExecutionEventPublisher()
    executor = PythonExecutorService(
        executor_id="executor://python-primary",
        publisher=publisher,
        handlers={"python.parse_artifacts": lambda payload: payload},
    )

    executor.consume(
        topic="execution.requests",
        payload={
            "request_id": "req-3b",
            "workflow_id": "wf-3b",
            "step_id": "dispatch-embed",
            "executor_id": "executor://ml-primary",
            "executor_kind": "ml-executor",
            "capability": "ml.embed",
            "policy": {},
            "constraints": {},
            "payload": {"input": "hola"},
            "transport": {"kind": "redpanda", "request_topic": "execution.requests"},
        },
    )

    assert publisher.published == []


def test_shared_topic_execution_event_publisher_maps_shared_events_to_topic_family() -> None:
    from publisher import TopicExecutionEventPublisher
    from interacciones.schemas import ExecutionCompleted, ExecutionFailed, ExecutionProgress, OrchestrationTopicNames

    publisher = TopicExecutionEventPublisher(topics=_dev_topics())

    publisher.publish_progress(
        ExecutionProgress(
            request_id="req-4",
            workflow_id="wf-4",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            status="running",
            progress=0.1,
            message="started",
        )
    )
    publisher.publish_completed(
        ExecutionCompleted(
            request_id="req-4",
            workflow_id="wf-4",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            result={"documents": 1},
            artifacts=["fragment://doc-1"],
        )
    )
    publisher.publish_failed(
        ExecutionFailed(
            request_id="req-5",
            workflow_id="wf-5",
            step_id="dispatch-embed",
            executor_id="executor://ml-primary",
            error_code="timeout",
            error_message="timed out",
            retryable=True,
        )
    )

    assert publisher.messages == [
        {
            "kind": "progress",
            "topic": "dev.execution.progress",
            "payload": {
                "request_id": "req-4",
                "workflow_id": "wf-4",
                "step_id": "dispatch-parse",
                "executor_id": "executor://python-primary",
                "status": "running",
                "progress": 0.1,
                "message": "started",
                "stdout_lines": [],
                "stderr_lines": [],
                "details": {},
            },
        },
        {
            "kind": "completed",
            "topic": "dev.execution.results",
            "payload": {
                "request_id": "req-4",
                "workflow_id": "wf-4",
                "step_id": "dispatch-parse",
                "executor_id": "executor://python-primary",
                "result": {"documents": 1},
                "artifacts": ["fragment://doc-1"],
                "stdout_lines": [],
                "stderr_lines": [],
            },
        },
        {
            "kind": "failed",
            "topic": "dev.execution.results",
            "payload": {
                "request_id": "req-5",
                "workflow_id": "wf-5",
                "step_id": "dispatch-embed",
                "executor_id": "executor://ml-primary",
                "error_code": "timeout",
                "error_message": "timed out",
                "retryable": True,
                "stdout_lines": [],
                "stderr_lines": [],
                "details": {},
            },
        },
    ]


def test_shared_queue_executor_consumer_accepts_only_execution_request_topic() -> None:
    from consumer import SharedQueueExecutorConsumer
    _, InMemoryExecutionEventPublisher, PythonExecutorService = _imports()

    publisher = InMemoryExecutionEventPublisher()
    executor = PythonExecutorService(
        executor_id="executor://python-primary",
        publisher=publisher,
        handlers={"python.parse_artifacts": lambda payload: {"documents": [payload["input"]]}},
    )
    consumer = SharedQueueExecutorConsumer(executor)

    consumer.consume(
        topic="execution.requests",
        payload={
            "request_id": "req-6",
            "workflow_id": "wf-6",
            "step_id": "dispatch-parse",
            "executor_id": "executor://python-primary",
            "executor_kind": "python-executor",
            "capability": "python.parse_artifacts",
            "policy": {},
            "constraints": {},
            "payload": {"input": "hola"},
            "transport": {"kind": "redpanda", "request_topic": "execution.requests"},
        },
    )

    assert [item["kind"] for item in publisher.published] == ["progress", "completed"]

    try:
        consumer.consume(topic="execution.progress", payload={})
    except ValueError as exc:
        assert str(exc) == "unsupported executor topic: execution.progress"
    else:
        raise AssertionError("expected unknown executor topic to be rejected")


def test_broker_backed_executor_runtime_polls_request_topic_and_publishes_shared_events() -> None:
    from broker_runtime import BrokerBackedExecutorRuntime
    from consumer import SharedQueueExecutorConsumer
    from publisher import TopicExecutionEventPublisher
    from interacciones.schemas import OrchestrationTopicNames

    _, _, PythonExecutorService = _imports()

    class FakeBroker:
        def __init__(self) -> None:
            self.polled_topics: list[str] = []
            self.messages = [
                (
                    "execution.requests",
                    {
                        "request_id": "req-7",
                        "workflow_id": "wf-7",
                        "step_id": "dispatch-parse",
                        "executor_id": "executor://python-primary",
                        "executor_kind": "python-executor",
                        "capability": "python.parse_artifacts",
                        "policy": {},
                        "constraints": {},
                        "payload": {"input": "hola"},
                        "transport": {"kind": "redpanda", "request_topic": "execution.requests"},
                    },
                )
            ]
            self.published: list[dict[str, object]] = []
            self.committed_topics: list[str] = []

        def poll(self, topic: str) -> tuple[str, dict] | None:
            self.polled_topics.append(topic)
            if not self.messages:
                return None
            return self.messages.pop(0)

        def publish(self, topic: str, payload: dict) -> None:
            self.published.append({"topic": topic, "payload": payload})

        def commit(self, topic: str) -> None:
            self.committed_topics.append(topic)

    broker = FakeBroker()
    publisher = TopicExecutionEventPublisher(topics=OrchestrationTopicNames())
    executor = PythonExecutorService(
        executor_id="executor://python-primary",
        publisher=publisher,
        handlers={"python.parse_artifacts": lambda payload: {"documents": [payload["input"].upper()]}},
    )
    runtime = BrokerBackedExecutorRuntime(
        broker=broker,
        consumer=SharedQueueExecutorConsumer(executor),
        publisher=publisher,
        topics=OrchestrationTopicNames(),
    )

    processed = runtime.run_once()

    assert processed is True
    assert broker.polled_topics == ["execution.requests"]
    assert broker.committed_topics == ["execution.requests"]
    assert broker.published == [
        {
            "topic": "execution.progress",
            "payload": {
                "request_id": "req-7",
                "workflow_id": "wf-7",
                "step_id": "dispatch-parse",
                "executor_id": "executor://python-primary",
                    "status": "running",
                    "progress": 0.0,
                    "message": "started",
                    "stdout_lines": [],
                    "stderr_lines": [],
                    "details": {},
                },
            },
        {
            "topic": "execution.results",
            "payload": {
                "request_id": "req-7",
                "workflow_id": "wf-7",
                "step_id": "dispatch-parse",
                    "executor_id": "executor://python-primary",
                    "result": {"documents": ["HOLA"]},
                    "artifacts": [],
                    "stdout_lines": [],
                    "stderr_lines": [],
                },
            },
        ]


def test_broker_backed_executor_runtime_returns_false_when_no_message_is_available() -> None:
    from broker_runtime import BrokerBackedExecutorRuntime
    from consumer import SharedQueueExecutorConsumer
    from publisher import TopicExecutionEventPublisher
    from interacciones.schemas import OrchestrationTopicNames

    _, _, PythonExecutorService = _imports()

    class EmptyBroker:
        def __init__(self) -> None:
            self.polled_topics: list[str] = []
            self.published: list[dict[str, object]] = []
            self.committed_topics: list[str] = []

        def poll(self, topic: str) -> None:
            self.polled_topics.append(topic)
            return None

        def publish(self, topic: str, payload: dict) -> None:
            self.published.append({"topic": topic, "payload": payload})

        def commit(self, topic: str) -> None:
            self.committed_topics.append(topic)

    broker = EmptyBroker()
    publisher = TopicExecutionEventPublisher(topics=OrchestrationTopicNames())
    executor = PythonExecutorService(
        executor_id="executor://python-primary",
        publisher=publisher,
        handlers={"python.parse_artifacts": lambda payload: {"documents": [payload["input"]]}},
    )
    runtime = BrokerBackedExecutorRuntime(
        broker=broker,
        consumer=SharedQueueExecutorConsumer(executor),
        publisher=publisher,
        topics=OrchestrationTopicNames(),
    )

    processed = runtime.run_once()

    assert processed is False
    assert broker.polled_topics == ["execution.requests"]
    assert broker.committed_topics == []
    assert broker.published == []


def test_redpanda_broker_polls_subscribed_request_topic_and_decodes_json_payload() -> None:
    from redpanda_broker import RedpandaExecutorBroker
    from interacciones.schemas import OrchestrationTopicNames

    class FakeMessage:
        def __init__(self, topic: str, payload: bytes) -> None:
            self._topic = topic
            self._payload = payload

        def topic(self) -> str:
            return self._topic

        def value(self) -> bytes:
            return self._payload

        def error(self) -> None:
            return None

    class FakeConsumer:
        def __init__(self) -> None:
            self.subscriptions: list[list[str]] = []

        def subscribe(self, topics: list[str]) -> None:
            self.subscriptions.append(topics)

        def poll(self, timeout: float) -> FakeMessage:
            assert timeout == 1.0
            return FakeMessage("dev.execution.requests", b'{"request_id":"req-8","workflow_id":"wf-8"}')

    class FakeProducer:
        def produce(self, topic: str, value: bytes) -> None:
            raise AssertionError("produce should not be called")

        def flush(self, timeout: float | None = None) -> None:
            raise AssertionError("flush should not be called")

    broker = RedpandaExecutorBroker(
        topics=_dev_topics(),
        consumer_factory=lambda group_id: FakeConsumer(),
        producer_factory=lambda: FakeProducer(),
        consumer_group_id="executor-python",
    )

    message = broker.poll("dev.execution.requests")

    assert broker._consumer.subscriptions == [["dev.execution.requests"]]
    assert message == ("dev.execution.requests", {"request_id": "req-8", "workflow_id": "wf-8"})


def test_redpanda_broker_publishes_json_payload_and_flushes_producer() -> None:
    from redpanda_broker import RedpandaExecutorBroker
    from interacciones.schemas import OrchestrationTopicNames

    class FakeConsumer:
        def subscribe(self, topics: list[str]) -> None:
            return None

        def poll(self, timeout: float) -> None:
            return None

    class FakeProducer:
        def __init__(self) -> None:
            self.produced: list[tuple[str, bytes]] = []
            self.flushed: list[float | None] = []

        def produce(self, topic: str, value: bytes) -> None:
            self.produced.append((topic, value))

        def flush(self, timeout: float | None = None) -> None:
            self.flushed.append(timeout)

    producer = FakeProducer()
    broker = RedpandaExecutorBroker(
        topics=OrchestrationTopicNames(),
        consumer_factory=lambda group_id: FakeConsumer(),
        producer_factory=lambda: producer,
        consumer_group_id="executor-python",
    )

    broker.publish("execution.results", {"request_id": "req-9", "status": "ok"})

    assert producer.produced == [("execution.results", b'{"request_id": "req-9", "status": "ok"}')]
    assert producer.flushed == [1.0]


def test_build_python_executor_runtime_assembles_redpanda_broker_consumer_and_topic_publisher() -> None:
    from shell import build_python_executor_runtime
    from interacciones.schemas import OrchestrationTopicNames

    class FakeConsumer:
        def subscribe(self, topics: list[str]) -> None:
            return None

        def poll(self, timeout: float) -> None:
            return None

    class FakeProducer:
        def produce(self, topic: str, value: bytes) -> None:
            return None

        def flush(self, timeout: float | None = None) -> None:
            return None

    runtime = build_python_executor_runtime(
        executor_id="executor://python-primary",
        handlers={"python.parse_artifacts": lambda payload: {"documents": [payload["input"]]}},
        consumer_group_id="python-executor-group",
        topics=_dev_topics(),
        consumer_factory=lambda group_id: FakeConsumer(),
        producer_factory=lambda: FakeProducer(),
    )

    assert runtime._topics.execution_requests == "dev.execution.requests"
    assert runtime._broker.__class__.__name__ == "RedpandaExecutorBroker"
    assert runtime._consumer.__class__.__name__ == "SharedQueueExecutorConsumer"
    assert runtime._publisher.__class__.__name__ == "TopicExecutionEventPublisher"
    assert runtime._consumer._executor._executor_id == "executor://python-primary"
    assert runtime._consumer._executor._executor_kind == "python-executor"


def test_build_ml_executor_runtime_assembles_ml_executor_variant() -> None:
    from shell import build_ml_executor_runtime

    class FakeConsumer:
        def subscribe(self, topics: list[str]) -> None:
            return None

        def poll(self, timeout: float) -> None:
            return None

    class FakeProducer:
        def produce(self, topic: str, value: bytes) -> None:
            return None

        def flush(self, timeout: float | None = None) -> None:
            return None

    runtime = build_ml_executor_runtime(
        executor_id="executor://ml-primary",
        handlers={"ml.embed": lambda payload: {"embedding": [1.0]}},
        consumer_group_id="ml-executor-group",
        consumer_factory=lambda group_id: FakeConsumer(),
        producer_factory=lambda: FakeProducer(),
    )

    assert runtime._consumer._executor._executor_id == "executor://ml-primary"
    assert runtime._consumer._executor._executor_kind == "ml-executor"


def test_broker_backed_executor_runtime_run_forever_stops_after_max_iterations() -> None:
    from broker_runtime import BrokerBackedExecutorRuntime
    from consumer import SharedQueueExecutorConsumer
    from publisher import TopicExecutionEventPublisher
    from interacciones.schemas import OrchestrationTopicNames

    _, _, PythonExecutorService = _imports()

    class SequenceBroker:
        def __init__(self) -> None:
            self.polled_topics: list[str] = []
            self.messages = [
                (
                    "execution.requests",
                    {
                        "request_id": "req-10",
                        "workflow_id": "wf-10",
                        "step_id": "dispatch-parse",
                        "executor_id": "executor://python-primary",
                        "executor_kind": "python-executor",
                        "capability": "python.parse_artifacts",
                        "policy": {},
                        "constraints": {},
                        "payload": {"input": "hola"},
                        "transport": {"kind": "redpanda", "request_topic": "execution.requests"},
                    },
                ),
                None,
                None,
            ]
            self.published: list[dict[str, object]] = []
            self.committed_topics: list[str] = []

        def poll(self, topic: str):
            self.polled_topics.append(topic)
            return self.messages.pop(0)

        def publish(self, topic: str, payload: dict) -> None:
            self.published.append({"topic": topic, "payload": payload})

        def commit(self, topic: str) -> None:
            self.committed_topics.append(topic)

    broker = SequenceBroker()
    publisher = TopicExecutionEventPublisher(topics=OrchestrationTopicNames())
    executor = PythonExecutorService(
        executor_id="executor://python-primary",
        publisher=publisher,
        handlers={"python.parse_artifacts": lambda payload: {"documents": [payload["input"].upper()]}},
    )
    runtime = BrokerBackedExecutorRuntime(
        broker=broker,
        consumer=SharedQueueExecutorConsumer(executor),
        publisher=publisher,
        topics=OrchestrationTopicNames(),
    )

    iterations = runtime.run_forever(max_iterations=3)

    assert iterations == 3
    assert broker.polled_topics == ["execution.requests", "execution.requests", "execution.requests"]
    assert broker.committed_topics == ["execution.requests"]
    assert [item["topic"] for item in broker.published] == ["execution.progress", "execution.results"]


def test_broker_backed_executor_runtime_run_forever_honors_should_continue_callback() -> None:
    from broker_runtime import BrokerBackedExecutorRuntime
    from consumer import SharedQueueExecutorConsumer
    from publisher import TopicExecutionEventPublisher
    from interacciones.schemas import OrchestrationTopicNames

    _, _, PythonExecutorService = _imports()

    class IdleBroker:
        def __init__(self) -> None:
            self.polled_topics: list[str] = []

        def poll(self, topic: str):
            self.polled_topics.append(topic)
            return None

        def publish(self, topic: str, payload: dict) -> None:
            raise AssertionError("publish should not be called")

    broker = IdleBroker()
    publisher = TopicExecutionEventPublisher(topics=OrchestrationTopicNames())
    executor = PythonExecutorService(
        executor_id="executor://python-primary",
        publisher=publisher,
        handlers={"python.parse_artifacts": lambda payload: payload},
    )
    runtime = BrokerBackedExecutorRuntime(
        broker=broker,
        consumer=SharedQueueExecutorConsumer(executor),
        publisher=publisher,
        topics=OrchestrationTopicNames(),
    )

    seen: list[int] = []

    def should_continue(iteration: int) -> bool:
        seen.append(iteration)
        return iteration < 2

    iterations = runtime.run_forever(should_continue=should_continue)

    assert iterations == 2
    assert seen == [0, 1, 2]
    assert broker.polled_topics == ["execution.requests", "execution.requests"]


def test_run_python_executor_builds_runtime_and_invokes_run_forever() -> None:
    from python_executor_entry import run_python_executor

    captured: dict[str, object] = {}

    class FakeRuntime:
        def run_forever(self, *, max_iterations=None, should_continue=None):
            captured["max_iterations"] = max_iterations
            captured["should_continue"] = should_continue
            return 7

    def fake_builder(**kwargs):
        captured["builder_kwargs"] = kwargs
        return FakeRuntime()

    iterations = run_python_executor(
        handlers={"python.parse_artifacts": lambda payload: payload},
        executor_id="executor://python-primary",
        consumer_group_id="python-executor-group",
        max_iterations=3,
        build_runtime=fake_builder,
    )

    assert iterations == 7
    builder_kwargs = captured["builder_kwargs"]
    assert isinstance(builder_kwargs, dict)
    assert builder_kwargs["executor_id"] == "executor://python-primary"
    assert builder_kwargs["consumer_group_id"] == "python-executor-group"
    assert builder_kwargs["topics"] is None
    assert builder_kwargs["consumer_factory"] is None
    assert builder_kwargs["producer_factory"] is None
    assert "python.parse_artifacts" in builder_kwargs["handlers"]
    assert captured["max_iterations"] == 3
    assert captured["should_continue"] is None


def test_run_ml_executor_builds_runtime_and_invokes_run_forever() -> None:
    from ml_executor_entry import run_ml_executor

    captured: dict[str, object] = {}

    class FakeRuntime:
        def run_forever(self, *, max_iterations=None, should_continue=None):
            captured["max_iterations"] = max_iterations
            captured["should_continue"] = should_continue
            return 5

    def fake_builder(**kwargs):
        captured["builder_kwargs"] = kwargs
        return FakeRuntime()

    def should_continue(iteration: int) -> bool:
        return iteration < 2

    iterations = run_ml_executor(
        handlers={"ml.embed": lambda payload: payload},
        executor_id="executor://ml-primary",
        consumer_group_id="ml-executor-group",
        should_continue=should_continue,
        build_runtime=fake_builder,
    )

    assert iterations == 5
    builder_kwargs = captured["builder_kwargs"]
    assert isinstance(builder_kwargs, dict)
    assert builder_kwargs["executor_id"] == "executor://ml-primary"
    assert builder_kwargs["consumer_group_id"] == "ml-executor-group"
    assert builder_kwargs["topics"] is None
    assert builder_kwargs["consumer_factory"] is None
    assert builder_kwargs["producer_factory"] is None
    assert "ml.embed" in builder_kwargs["handlers"]
    assert captured["max_iterations"] is None
    assert captured["should_continue"] is should_continue


def test_broker_backed_executor_runtime_publishes_failure_result_when_handler_raises() -> None:
    from broker_runtime import BrokerBackedExecutorRuntime
    from consumer import SharedQueueExecutorConsumer
    from publisher import TopicExecutionEventPublisher
    from interacciones.schemas import OrchestrationTopicNames

    _, _, PythonExecutorService = _imports()

    class FailureBroker:
        def __init__(self) -> None:
            self.polled_topics: list[str] = []
            self.published: list[dict[str, object]] = []
            self.committed_topics: list[str] = []

        def poll(self, topic: str):
            self.polled_topics.append(topic)
            return (
                "execution.requests",
                {
                    "request_id": "req-11",
                    "workflow_id": "wf-11",
                    "step_id": "dispatch-parse",
                    "executor_id": "executor://python-primary",
                    "executor_kind": "python-executor",
                    "capability": "python.parse_artifacts",
                    "policy": {},
                    "constraints": {},
                    "payload": {"input": "hola"},
                    "transport": {"kind": "redpanda", "request_topic": "execution.requests"},
                },
            )

        def publish(self, topic: str, payload: dict) -> None:
            self.published.append({"topic": topic, "payload": payload})

        def commit(self, topic: str) -> None:
            self.committed_topics.append(topic)

    broker = FailureBroker()
    publisher = TopicExecutionEventPublisher(topics=OrchestrationTopicNames())
    executor = PythonExecutorService(
        executor_id="executor://python-primary",
        publisher=publisher,
        handlers={"python.parse_artifacts": lambda payload: (_ for _ in ()).throw(RuntimeError("boom"))},
    )
    runtime = BrokerBackedExecutorRuntime(
        broker=broker,
        consumer=SharedQueueExecutorConsumer(executor),
        publisher=publisher,
        topics=OrchestrationTopicNames(),
    )

    processed = runtime.run_once()

    assert processed is True
    assert broker.polled_topics == ["execution.requests"]
    assert broker.committed_topics == ["execution.requests"]
    assert broker.published == [
        {
            "topic": "execution.progress",
            "payload": {
                "request_id": "req-11",
                "workflow_id": "wf-11",
                "step_id": "dispatch-parse",
                "executor_id": "executor://python-primary",
                    "status": "running",
                    "progress": 0.0,
                    "message": "started",
                    "stdout_lines": [],
                    "stderr_lines": [],
                    "details": {},
                },
            },
        {
            "topic": "execution.results",
            "payload": {
                "request_id": "req-11",
                "workflow_id": "wf-11",
                "step_id": "dispatch-parse",
                "executor_id": "executor://python-primary",
                    "error_code": "execution_error",
                    "error_message": "boom",
                    "retryable": False,
                    "stdout_lines": [],
                    "stderr_lines": [],
                    "details": {},
                },
            },
        ]


def test_python_executor_entry_main_reads_env_and_delegates_to_runner() -> None:
    from python_executor_entry import main

    captured: dict[str, object] = {}

    def fake_load_handlers(module_name: str, attr_name: str):
        captured["module_name"] = module_name
        captured["attr_name"] = attr_name
        return {"python.parse_artifacts": lambda payload: payload}

    def fake_run(**kwargs):
        captured["run_kwargs"] = kwargs
        return 9

    iterations = main(
        env={
            "IKAM_EXECUTOR_HANDLERS_MODULE": "demo.handlers.python",
            "IKAM_EXECUTOR_HANDLERS_ATTR": "PYTHON_HANDLERS",
            "IKAM_EXECUTOR_ID": "executor://python-special",
            "IKAM_EXECUTOR_CONSUMER_GROUP_ID": "python-group-special",
            "IKAM_EXECUTOR_MAX_ITERATIONS": "4",
        },
        load_handlers=fake_load_handlers,
        run_executor=fake_run,
    )

    assert iterations == 9
    assert captured["module_name"] == "demo.handlers.python"
    assert captured["attr_name"] == "PYTHON_HANDLERS"
    run_kwargs = captured["run_kwargs"]
    assert isinstance(run_kwargs, dict)
    assert run_kwargs["executor_id"] == "executor://python-special"
    assert run_kwargs["consumer_group_id"] == "python-group-special"
    assert run_kwargs["max_iterations"] == 4
    assert "python.parse_artifacts" in run_kwargs["handlers"]


def test_ml_executor_entry_main_reads_env_and_delegates_to_runner() -> None:
    from ml_executor_entry import main

    captured: dict[str, object] = {}

    def fake_load_handlers(module_name: str, attr_name: str):
        captured["module_name"] = module_name
        captured["attr_name"] = attr_name
        return {"ml.embed": lambda payload: payload}

    def fake_run(**kwargs):
        captured["run_kwargs"] = kwargs
        return 6

    iterations = main(
        env={
            "IKAM_EXECUTOR_HANDLERS_MODULE": "demo.handlers.ml",
            "IKAM_EXECUTOR_HANDLERS_ATTR": "ML_HANDLERS",
            "IKAM_EXECUTOR_ID": "executor://ml-special",
            "IKAM_EXECUTOR_CONSUMER_GROUP_ID": "ml-group-special",
        },
        load_handlers=fake_load_handlers,
        run_executor=fake_run,
    )

    assert iterations == 6
    assert captured["module_name"] == "demo.handlers.ml"
    assert captured["attr_name"] == "ML_HANDLERS"
    run_kwargs = captured["run_kwargs"]
    assert isinstance(run_kwargs, dict)
    assert run_kwargs["executor_id"] == "executor://ml-special"
    assert run_kwargs["consumer_group_id"] == "ml-group-special"
    assert run_kwargs["max_iterations"] is None
    assert "ml.embed" in run_kwargs["handlers"]


def test_python_executor_entry_module_execution_invokes_main() -> None:
    result = subprocess.run(
        [sys.executable, "-m", "python_executor_entry"],
        capture_output=True,
        text=True,
        env={"PYTHONPATH": EXECUTOR_PYTHONPATH},
    )

    assert result.returncode != 0
    assert "IKAM_EXECUTOR_HANDLERS_MODULE" in result.stderr


def test_ml_executor_entry_module_execution_invokes_main() -> None:
    result = subprocess.run(
        [sys.executable, "-m", "ml_executor_entry"],
        capture_output=True,
        text=True,
        env={"PYTHONPATH": EXECUTOR_PYTHONPATH},
    )

    assert result.returncode != 0
    assert "IKAM_EXECUTOR_HANDLERS_MODULE" in result.stderr
