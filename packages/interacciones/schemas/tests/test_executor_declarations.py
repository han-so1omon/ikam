"""Contract tests for executor declaration schemas."""

import pytest
from pydantic import ValidationError

from interacciones.schemas import ExecutorDeclaration


def test_executor_declaration_round_trip() -> None:
    declaration = ExecutorDeclaration(
        executor_id="executor://python-primary",
        executor_kind="python-executor",
        capabilities=["python.chunk", "python.transform"],
        policy_support=["latency_tier", "cost_tier"],
        transport={"kind": "redpanda", "request_topic": "execution.requests"},
        runtime={"language": "python", "version": "3.11"},
        concurrency={"max_inflight": 4},
        batching={"max_batch_size": 16},
        health={"readiness_path": "/health"},
    )

    dumped = declaration.model_dump(mode="json")

    assert dumped == {
        "executor_id": "executor://python-primary",
        "executor_kind": "python-executor",
        "capabilities": ["python.chunk", "python.transform"],
        "policy_support": ["latency_tier", "cost_tier"],
        "transport": {"kind": "redpanda", "request_topic": "execution.requests"},
        "runtime": {"language": "python", "version": "3.11"},
        "concurrency": {"max_inflight": 4},
        "batching": {"max_batch_size": 16},
        "health": {"readiness_path": "/health"},
    }


def test_executor_declaration_requires_capabilities() -> None:
    with pytest.raises(ValidationError):
        ExecutorDeclaration(
            executor_id="executor://python-primary",
            executor_kind="python-executor",
            capabilities=[],
            policy_support=["latency_tier"],
            transport={"kind": "redpanda"},
            runtime={"language": "python"},
            concurrency={"max_inflight": 1},
            batching={"max_batch_size": 1},
            health={"readiness_path": "/health"},
        )


def test_executor_declaration_requires_policy_support_transport_and_runtime() -> None:
    with pytest.raises(ValidationError):
        ExecutorDeclaration.model_validate(
            {
                "executor_id": "executor://ml-primary",
                "executor_kind": "ml-executor",
                "capabilities": ["ml.embed"],
            }
        )


def test_executor_declaration_requires_extended_runtime_fields() -> None:
    with pytest.raises(ValidationError):
        ExecutorDeclaration.model_validate(
            {
                "executor_id": "executor://ml-primary",
                "executor_kind": "ml-executor",
                "capabilities": ["ml.embed"],
                "policy_support": ["latency_tier"],
                "transport": {"kind": "redpanda"},
                "runtime": {"framework": "pytorch"},
            }
        )


def test_executor_declaration_rejects_empty_transport_runtime_and_health_sections() -> None:
    with pytest.raises(ValidationError):
        ExecutorDeclaration.model_validate(
            {
                "executor_id": "executor://python-primary",
                "executor_kind": "python-executor",
                "capabilities": ["python.transform"],
                "policy_support": ["cost_tier"],
                "transport": {},
                "runtime": {},
                "concurrency": {},
                "batching": {},
                "health": {},
            }
        )
