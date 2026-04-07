import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from ikam_perf_report.executor_runtime_kafka import (
    decode_message,
    encode_message,
    poll_json_messages,
    poll_json_topic_messages,
    wait_for_assignment,
)


def test_encode_message_serializes_executor_runtime_payload() -> None:
    payload = {
        "request_id": "req-1",
        "workflow_id": "wf-1",
        "step_id": "dispatch-parse",
        "executor_id": "executor://python-primary",
        "status": "running",
    }

    encoded = encode_message(payload)

    assert json.loads(encoded.decode("utf-8")) == payload


def test_decode_message_deserializes_executor_runtime_payload() -> None:
    message = b'{"request_id": "req-2", "result": {"documents": 1}}'

    decoded = decode_message(message)

    assert decoded == {"request_id": "req-2", "result": {"documents": 1}}


def test_poll_json_messages_collects_matching_executor_events() -> None:
    class FakeMessage:
        def __init__(self, payload: bytes | None, error=None) -> None:
            self._payload = payload
            self._error = error

        def value(self):
            return self._payload

        def error(self):
            return self._error

    class FakeConsumer:
        def __init__(self) -> None:
            self.calls = 0

        def poll(self, timeout: float):
            self.calls += 1
            if self.calls == 1:
                return FakeMessage(b'{"request_id": "req-3", "status": "running"}')
            if self.calls == 2:
                return FakeMessage(b'{"request_id": "req-3", "result": {"documents": 1}}')
            return None

    consumer = FakeConsumer()

    messages = poll_json_messages(
        consumer,
        limit=2,
        poll_timeout=0.1,
        predicate=lambda payload: payload.get("request_id") == "req-3",
    )

    assert messages == [
        {"request_id": "req-3", "status": "running"},
        {"request_id": "req-3", "result": {"documents": 1}},
    ]


def test_poll_json_messages_skips_empty_polls_and_non_matching_messages() -> None:
    class FakeMessage:
        def __init__(self, payload: bytes | None, error=None) -> None:
            self._payload = payload
            self._error = error

        def value(self):
            return self._payload

        def error(self):
            return self._error

    class FakeConsumer:
        def __init__(self) -> None:
            self.calls = 0

        def poll(self, timeout: float):
            self.calls += 1
            if self.calls == 1:
                return None
            if self.calls == 2:
                return FakeMessage(b'{"request_id": "other"}')
            if self.calls == 3:
                return FakeMessage(b'{"request_id": "req-4", "error_code": "timeout"}')
            return None

    consumer = FakeConsumer()

    messages = poll_json_messages(
        consumer,
        limit=1,
        poll_timeout=0.1,
        max_polls=4,
        predicate=lambda payload: payload.get("request_id") == "req-4",
    )

    assert messages == [{"request_id": "req-4", "error_code": "timeout"}]


def test_poll_json_topic_messages_collects_topics_with_matching_executor_events() -> None:
    class FakeMessage:
        def __init__(self, topic: str, payload: bytes | None, error=None) -> None:
            self._topic = topic
            self._payload = payload
            self._error = error

        def topic(self):
            return self._topic

        def value(self):
            return self._payload

        def error(self):
            return self._error

    class FakeConsumer:
        def __init__(self) -> None:
            self.calls = 0

        def poll(self, timeout: float):
            self.calls += 1
            if self.calls == 1:
                return FakeMessage("workflow.events", b'{"request_id": "req-5", "status": "queued"}')
            if self.calls == 2:
                return FakeMessage("execution.progress", b'{"request_id": "req-5", "status": "running"}')
            if self.calls == 3:
                return FakeMessage("execution.results", b'{"request_id": "req-5", "result": {"documents": 1}}')
            return None

    consumer = FakeConsumer()

    messages = poll_json_topic_messages(
        consumer,
        limit=3,
        poll_timeout=0.1,
        predicate=lambda payload: payload.get("request_id") == "req-5",
    )

    assert messages == [
        ("workflow.events", {"request_id": "req-5", "status": "queued"}),
        ("execution.progress", {"request_id": "req-5", "status": "running"}),
        ("execution.results", {"request_id": "req-5", "result": {"documents": 1}}),
    ]


def test_wait_for_assignment_polls_until_consumer_has_assignment() -> None:
    class FakeConsumer:
        def __init__(self) -> None:
            self.calls = 0

        def assignment(self):
            return [] if self.calls < 2 else ["execution.results"]

        def poll(self, timeout: float):
            self.calls += 1
            return None

    consumer = FakeConsumer()

    assigned = wait_for_assignment(consumer, poll_timeout=0.1, max_polls=4)

    assert assigned is True
    assert consumer.calls == 2


def test_wait_for_assignment_returns_false_when_consumer_never_assigns() -> None:
    class FakeConsumer:
        def __init__(self) -> None:
            self.calls = 0

        def assignment(self):
            return []

        def poll(self, timeout: float):
            self.calls += 1
            return None

    consumer = FakeConsumer()

    assigned = wait_for_assignment(consumer, poll_timeout=0.1, max_polls=3)

    assert assigned is False
    assert consumer.calls == 3
