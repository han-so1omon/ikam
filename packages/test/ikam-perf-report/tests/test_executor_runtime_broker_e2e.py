from __future__ import annotations

import os
from pathlib import Path
import sys
from uuid import uuid4

import pytest


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from ikam_perf_report.executor_runtime_kafka import (
    encode_message,
    poll_json_messages,
    poll_json_topic_messages,
    wait_for_assignment,
)


pytestmark = [
    pytest.mark.skipif(
        not os.getenv("ENABLE_EXECUTOR_RUNTIME_KAFKA_E2E_TESTS"),
        reason="Set ENABLE_EXECUTOR_RUNTIME_KAFKA_E2E_TESTS=1 to run executor runtime Kafka E2E tests",
    ),
]


def _bootstrap_servers() -> str:
    return os.getenv("KAFKA_BOOTSTRAP_SERVERS", "localhost:19092")


def test_python_executor_runtime_emits_progress_and_result_events_over_broker() -> None:
    confluent_kafka = pytest.importorskip("confluent_kafka")

    producer = confluent_kafka.Producer({"bootstrap.servers": _bootstrap_servers()})
    consumer = confluent_kafka.Consumer(
        {
            "bootstrap.servers": _bootstrap_servers(),
            "group.id": f"executor-runtime-e2e-{uuid4()}",
            "auto.offset.reset": "earliest",
            "enable.auto.commit": False,
        }
    )
    consumer.subscribe(["execution.progress", "execution.results"])

    request_id = f"req-{uuid4()}"
    payload = {
        "request_id": request_id,
        "workflow_id": "wf-e2e-1",
        "step_id": "load-documents",
        "executor_id": "executor://python-primary",
        "executor_kind": "python-executor",
        "capability": "python.load_documents",
        "policy": {},
        "constraints": {},
        "payload": {"raw_bytes": "Hello LlamaIndex!"},
        "transport": {"kind": "redpanda", "request_topic": "execution.requests"},
    }

    try:
        assert wait_for_assignment(consumer, poll_timeout=0.1, max_polls=20)
        producer.produce("execution.requests", value=encode_message(payload))
        producer.flush(5.0)

        messages = poll_json_messages(
            consumer,
            limit=2,
            poll_timeout=1.0,
            max_polls=200,
            predicate=lambda item: item.get("request_id") == request_id,
        )
    finally:
        consumer.close()

    assert len(messages) == 2
    assert any(message.get("status") == "running" for message in messages)
    assert any("result" in message and "documents" in message["result"] for message in messages)


def test_ml_executor_runtime_emits_progress_and_result_events_over_broker() -> None:
    confluent_kafka = pytest.importorskip("confluent_kafka")

    producer = confluent_kafka.Producer({"bootstrap.servers": _bootstrap_servers()})
    consumer = confluent_kafka.Consumer(
        {
            "bootstrap.servers": _bootstrap_servers(),
            "group.id": f"executor-runtime-e2e-{uuid4()}",
            "auto.offset.reset": "earliest",
            "enable.auto.commit": False,
        }
    )
    consumer.subscribe(["execution.progress", "execution.results"])

    request_id = f"req-{uuid4()}"
    payload = {
        "request_id": request_id,
        "workflow_id": "wf-e2e-2",
        "step_id": "dispatch-embed",
        "executor_id": "executor://ml-primary",
        "executor_kind": "ml-executor",
        "capability": "ml.embed",
        "policy": {},
        "constraints": {},
        "payload": {"input": "hola"},
        "transport": {"kind": "redpanda", "request_topic": "execution.requests"},
    }

    try:
        assert wait_for_assignment(consumer, poll_timeout=0.1, max_polls=20)
        producer.produce("execution.requests", value=encode_message(payload))
        producer.flush(5.0)

        messages = poll_json_messages(
            consumer,
            limit=2,
            poll_timeout=1.0,
            max_polls=200,
            predicate=lambda item: item.get("request_id") == request_id,
        )
    finally:
        consumer.close()

    assert len(messages) == 2
    assert any(message.get("status") == "running" for message in messages)
    assert any("result" in message and "embedding" in message["result"] for message in messages)


def test_python_executor_runtime_preserves_queued_handoff_alongside_later_broker_events() -> None:
    confluent_kafka = pytest.importorskip("confluent_kafka")

    producer = confluent_kafka.Producer({"bootstrap.servers": _bootstrap_servers()})
    consumer = confluent_kafka.Consumer(
        {
            "bootstrap.servers": _bootstrap_servers(),
            "group.id": f"executor-runtime-e2e-{uuid4()}",
            "auto.offset.reset": "earliest",
            "enable.auto.commit": False,
        }
    )
    consumer.subscribe(["workflow.events", "execution.progress", "execution.results"])

    request_id = f"req-{uuid4()}"
    queued = {
        "request_id": request_id,
        "workflow_id": "wf-e2e-queued-1",
        "step_id": "load-documents",
        "executor_id": "executor://python-primary",
        "executor_kind": "python-executor",
        "capability": "python.load_documents",
        "status": "queued",
    }
    request = {
        "request_id": request_id,
        "workflow_id": "wf-e2e-queued-1",
        "step_id": "load-documents",
        "executor_id": "executor://python-primary",
        "executor_kind": "python-executor",
        "capability": "python.load_documents",
        "policy": {},
        "constraints": {},
        "payload": {"raw_bytes": "Hello LlamaIndex!"},
        "transport": {"kind": "redpanda", "request_topic": "execution.requests"},
    }

    try:
        assert wait_for_assignment(consumer, poll_timeout=0.1, max_polls=20)
        producer.produce("workflow.events", value=encode_message(queued))
        producer.produce("execution.requests", value=encode_message(request))
        producer.flush(5.0)

        messages = poll_json_topic_messages(
            consumer,
            limit=3,
            poll_timeout=1.0,
            max_polls=300,
            predicate=lambda item: item.get("request_id") == request_id,
        )
    finally:
        consumer.close()

    assert len(messages) == 3
    assert ("workflow.events", queued) in messages
    assert any(topic == "execution.progress" and payload.get("status") == "running" for topic, payload in messages)
    assert any(
        topic == "execution.results" and "result" in payload and "documents" in payload["result"]
        for topic, payload in messages
    )
