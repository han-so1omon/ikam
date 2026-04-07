from __future__ import annotations

from pathlib import Path
import sys

import pytest


ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "packages/interacciones/graph/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))

from interacciones.graph.workflow.resolver import WorkflowExecutorResolver
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


def test_workflow_executor_resolver_matches_by_capability() -> None:
    resolver = WorkflowExecutorResolver(_declarations())
    request = ExecutionRequest(
        request_id="req-1",
        workflow_id="ingestion-early-steps",
        step_id="dispatch-parse",
        capability="python.parse_artifacts",
        policy={"cost_tier": "standard"},
        constraints={},
        payload={"input": "hello"},
    )

    resolved = resolver.resolve(request)

    assert resolved.executor_id == "executor://python-primary"


def test_workflow_executor_resolver_honors_direct_executor_override() -> None:
    resolver = WorkflowExecutorResolver(_declarations())
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

    resolved = resolver.resolve(request)

    assert resolved.executor_id == "executor://ml-primary"


def test_workflow_executor_resolver_rejects_unknown_capability() -> None:
    resolver = WorkflowExecutorResolver(_declarations())
    request = ExecutionRequest(
        request_id="req-3",
        workflow_id="ingestion-early-steps",
        step_id="dispatch-rerank",
        capability="ml.rerank",
        policy={},
        constraints={},
        payload={"input": "hello"},
    )

    with pytest.raises(ValueError, match="no executor declaration supports capability 'ml.rerank'"):
        resolver.resolve(request)
