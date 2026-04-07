from __future__ import annotations

from pathlib import Path
import sys
import types


ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/modelado/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/graph/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))

from interacciones.graph.workflow.bus import WorkflowBus
from interacciones.graph.workflow.event_handlers import WorkflowExecutionEventHandlers
from interacciones.graph.workflow.orchestrator import WorkflowOrchestrator
from interacciones.graph.workflow.state_store import InMemoryWorkflowStateStore
from interacciones.graph.workflow.topic_consumer import WorkflowTopicConsumer
from interacciones.schemas import ExecutionQueueRequest, ExecutionQueued, OrchestrationTopicNames
from interacciones.schemas.executors import ExecutorDeclaration
from modelado.environment_scope import EnvironmentScope
from modelado.executors import ExecutionDispatcher, ExecutionQueueBus
from modelado.operators.core import OperatorEnv


_DEV_TOPICS = OrchestrationTopicNames(
    execution_requests="dev.execution.requests",
    execution_progress="dev.execution.progress",
    execution_results="dev.execution.results",
    workflow_events="dev.workflow.events",
    approval_events="dev.approval.events",
    mcp_events="dev.mcp.events",
    acp_events="dev.acp.events",
)


def test_modelado_queue_publish_handoff_is_consumable_by_workflow_topic_consumer(
    monkeypatch,
) -> None:
    dispatcher = ExecutionDispatcher()
    workflow_bus = WorkflowBus(topics=_DEV_TOPICS)
    queue_bus = ExecutionQueueBus.from_workflow_bus(workflow_bus)
    store = InMemoryWorkflowStateStore()
    orchestrator = WorkflowOrchestrator(
        [
            ExecutorDeclaration(
                executor_id="executor://python-primary",
                executor_kind="python-executor",
                capabilities=["python.parse_artifacts"],
                policy_support=["cost_tier"],
                transport={"kind": "redpanda", "request_topic": "execution.requests"},
                runtime={"language": "python", "version": "3.11"},
                concurrency={"max_inflight": 4},
                batching={"max_batch_size": 16},
                health={"readiness_path": "/health"},
            )
        ],
        state_store=store,
    )
    consumer = WorkflowTopicConsumer(
        WorkflowExecutionEventHandlers(orchestrator),
        topics=_DEV_TOPICS,
    )

    monkeypatch.setattr(
        dispatcher,
        "build_execution_queue_request",
        lambda **_: ExecutionQueueRequest(
            request_id="req-handoff-1",
            workflow_id="wf-handoff-1",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            executor_kind="python-executor",
            capability="python.parse_artifacts",
            policy={"cost_tier": "standard"},
            constraints={},
            payload={"module": "modelado.executors.loaders"},
            transport={"kind": "redpanda", "request_topic": "execution.requests"},
        ),
    )

    queued = dispatcher.publish_execution_queue_request(
        transition_fragment_id="transition-fragment",
        fragment={"fragment_id": "doc-1"},
        params=types.SimpleNamespace(name="dispatch-parse", parameters={"input": "hola"}),
        env=OperatorEnv(
            seed=1,
            renderer_version="test",
            policy="test",
            env_scope=EnvironmentScope(ref="refs/heads/run/env-1"),
        ),
        bus=queue_bus,
    )

    workflow_topic, workflow_payload = queue_bus.messages[1]
    consumer.consume(workflow_topic, workflow_payload)
    consumer.consume(
        workflow_bus.topics.execution_progress,
        {
            "request_id": "req-handoff-1",
            "workflow_id": "wf-handoff-1",
            "step_id": "dispatch-parse",
            "executor_id": "executor://python-primary",
            "status": "running",
            "progress": 0.5,
            "message": "halfway",
        },
    )
    consumer.consume(
        workflow_bus.topics.execution_results,
        {
            "request_id": "req-handoff-1",
            "workflow_id": "wf-handoff-1",
            "step_id": "dispatch-parse",
            "executor_id": "executor://python-primary",
            "result": {"documents": 1},
            "artifacts": ["artifact://parsed"],
        },
    )

    assert queued == ExecutionQueued(
        request_id="req-handoff-1",
        workflow_id="wf-handoff-1",
        step_id="dispatch-parse",
        executor_id="executor://python-primary",
        executor_kind="python-executor",
        capability="python.parse_artifacts",
        status="queued",
    )
    assert (workflow_topic, workflow_payload) == (
        "dev.workflow.events",
        queued.model_dump(mode="json"),
    )
    assert store.get("wf-handoff-1") == {
        "workflow_id": "wf-handoff-1",
        "current_step": "dispatch-parse",
        "status": "completed",
        "executor_id": "executor://python-primary",
        "request_id": "req-handoff-1",
        "capability": "python.parse_artifacts",
        "executor_kind": "python-executor",
        "result": {"documents": 1},
        "artifacts": ["artifact://parsed"],
        "stdout_lines": [],
        "stderr_lines": [],
    }


def test_modelado_queue_publish_failure_handoff_is_consumable_by_workflow_topic_consumer(
    monkeypatch,
) -> None:
    dispatcher = ExecutionDispatcher()
    workflow_bus = WorkflowBus(topics=_DEV_TOPICS)
    queue_bus = ExecutionQueueBus.from_workflow_bus(workflow_bus)
    store = InMemoryWorkflowStateStore()
    orchestrator = WorkflowOrchestrator(
        [
            ExecutorDeclaration(
                executor_id="executor://python-primary",
                executor_kind="python-executor",
                capabilities=["python.parse_artifacts"],
                policy_support=["cost_tier"],
                transport={"kind": "redpanda", "request_topic": "execution.requests"},
                runtime={"language": "python", "version": "3.11"},
                concurrency={"max_inflight": 4},
                batching={"max_batch_size": 16},
                health={"readiness_path": "/health"},
            )
        ],
        state_store=store,
    )
    consumer = WorkflowTopicConsumer(
        WorkflowExecutionEventHandlers(orchestrator),
        topics=_DEV_TOPICS,
    )

    monkeypatch.setattr(
        dispatcher,
        "build_execution_queue_request",
        lambda **_: ExecutionQueueRequest(
            request_id="req-handoff-fail-1",
            workflow_id="wf-handoff-fail-1",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            executor_kind="python-executor",
            capability="python.parse_artifacts",
            policy={"cost_tier": "standard"},
            constraints={},
            payload={"module": "modelado.executors.loaders"},
            transport={"kind": "redpanda", "request_topic": "execution.requests"},
        ),
    )

    queued = dispatcher.publish_execution_queue_request(
        transition_fragment_id="transition-fragment",
        fragment={"fragment_id": "doc-1"},
        params=types.SimpleNamespace(name="dispatch-parse", parameters={"input": "hola"}),
        env=OperatorEnv(
            seed=1,
            renderer_version="test",
            policy="test",
            env_scope=EnvironmentScope(ref="refs/heads/run/env-1"),
        ),
        bus=queue_bus,
    )

    workflow_topic, workflow_payload = queue_bus.messages[1]
    consumer.consume(workflow_topic, workflow_payload)
    consumer.consume(
        workflow_bus.topics.execution_progress,
        {
            "request_id": "req-handoff-fail-1",
            "workflow_id": "wf-handoff-fail-1",
            "step_id": "dispatch-parse",
            "executor_id": "executor://python-primary",
            "status": "running",
            "progress": 0.5,
            "message": "halfway",
        },
    )
    consumer.consume(
        workflow_bus.topics.execution_results,
        {
            "request_id": "req-handoff-fail-1",
            "workflow_id": "wf-handoff-fail-1",
            "step_id": "dispatch-parse",
            "executor_id": "executor://python-primary",
            "error_code": "executor_timeout",
            "error_message": "timed out",
            "retryable": False,
        },
    )

    assert queued == ExecutionQueued(
        request_id="req-handoff-fail-1",
        workflow_id="wf-handoff-fail-1",
        step_id="dispatch-parse",
        executor_id="executor://python-primary",
        executor_kind="python-executor",
        capability="python.parse_artifacts",
        status="queued",
    )
    assert (workflow_topic, workflow_payload) == (
        "dev.workflow.events",
        queued.model_dump(mode="json"),
    )
    assert store.get("wf-handoff-fail-1") == {
        "workflow_id": "wf-handoff-fail-1",
        "current_step": "dispatch-parse",
        "status": "failed",
        "executor_id": "executor://python-primary",
        "request_id": "req-handoff-fail-1",
        "capability": "python.parse_artifacts",
        "executor_kind": "python-executor",
        "error_code": "executor_timeout",
        "error_message": "timed out",
        "retryable": False,
        "stdout_lines": [],
        "stderr_lines": [],
    }
