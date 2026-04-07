from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "packages/interacciones/graph/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))

from interacciones.graph.workflow.dispatcher import WorkflowExecutionDispatcher
from interacciones.schemas.execution import ExecutionRequest, ResolutionMode
from interacciones.schemas.executors import ExecutorDeclaration


def _declarations() -> list[ExecutorDeclaration]:
    return [
        ExecutorDeclaration(
            executor_id="executor://python-primary",
            executor_kind="python-executor",
            capabilities=["python.parse_artifacts", "python.transform"],
            policy_support=["cost_tier"],
            transport={"kind": "redpanda", "request_topic": "execution.requests"},
            runtime={"language": "python", "version": "3.11"},
            concurrency={"max_inflight": 4},
            batching={"max_batch_size": 16},
            health={"readiness_path": "/health"},
        ),
        ExecutorDeclaration(
            executor_id="executor://ml-primary",
            executor_kind="ml-executor",
            capabilities=["ml.embed"],
            policy_support=["latency_tier"],
            transport={"kind": "redpanda", "request_topic": "execution.requests"},
            runtime={"framework": "pytorch", "device": "cuda"},
            concurrency={"max_inflight": 2},
            batching={"max_batch_size": 64},
            health={"readiness_path": "/health"},
        ),
    ]


def test_workflow_execution_dispatcher_publishes_request_on_resolved_execution_topic() -> None:
    dispatcher = WorkflowExecutionDispatcher(_declarations())
    request = ExecutionRequest(
        request_id="req-1",
        workflow_id="ingestion-early-steps",
        step_id="dispatch-parse",
        capability="python.parse_artifacts",
        policy={"cost_tier": "standard"},
        constraints={},
        payload={"input": "hello"},
    )

    published = dispatcher.dispatch(request)

    assert published.topic == "execution.requests"
    assert published.payload == {
        "request_id": "req-1",
        "workflow_id": "ingestion-early-steps",
        "step_id": "dispatch-parse",
        "executor_id": "executor://python-primary",
        "executor_kind": "python-executor",
        "capability": "python.parse_artifacts",
        "policy": {"cost_tier": "standard"},
        "constraints": {},
        "payload": {"input": "hello"},
        "transport": {"kind": "redpanda", "request_topic": "execution.requests"},
    }


def test_workflow_execution_dispatcher_honors_direct_executor_override() -> None:
    dispatcher = WorkflowExecutionDispatcher(_declarations())
    request = ExecutionRequest(
        request_id="req-2",
        workflow_id="embed-flow",
        step_id="dispatch-embed",
        capability="ml.embed",
        policy={"latency_tier": "interactive"},
        constraints={},
        payload={"input": "hello"},
        resolution_mode=ResolutionMode.DIRECT_EXECUTOR_REF,
        direct_executor_ref="executor://ml-primary",
    )

    published = dispatcher.dispatch(request)

    assert published.payload["executor_id"] == "executor://ml-primary"
    assert published.payload["executor_kind"] == "ml-executor"
