from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/modelado/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/graph/src"))

from interacciones.graph.workflow.bus import WorkflowBus
from interacciones.schemas import ExecutionQueueRequest, ExecutionQueued, OrchestrationTopicNames
from modelado.environment_scope import EnvironmentScope
from modelado.executors.bus import ExecutionQueueBus, attach_execution_queue_bus
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


def test_execution_queue_bus_bridges_to_workflow_bus_request_channel() -> None:
    bus = ExecutionQueueBus.from_workflow_bus(WorkflowBus())

    bus.publish_request(
        ExecutionQueueRequest(
            request_id="req-1",
            workflow_id="wf-1",
            step_id="step-1",
            executor_id="executor://python-primary",
            executor_kind="python-executor",
            capability="python.parse_artifacts",
            policy={},
            constraints={},
            payload={"module": "modelado.executors.loaders"},
            transport={"kind": "redpanda", "request_topic": "execution.requests"},
        )
    )

    assert bus.messages == [
        (
            "execution.requests",
            {
                "request_id": "req-1",
                "workflow_id": "wf-1",
                "step_id": "step-1",
                "executor_id": "executor://python-primary",
                "executor_kind": "python-executor",
                "capability": "python.parse_artifacts",
                "policy": {},
                "constraints": {},
                "payload": {"module": "modelado.executors.loaders"},
                "transport": {"kind": "redpanda", "request_topic": "execution.requests"},
            },
        ),
        (
            "workflow.events",
            ExecutionQueued(
                request_id="req-1",
                workflow_id="wf-1",
                step_id="step-1",
                executor_id="executor://python-primary",
                executor_kind="python-executor",
                capability="python.parse_artifacts",
            ).model_dump(mode="json"),
        )
    ]


def test_execution_queue_bus_bridges_prefixed_workflow_bus_topics() -> None:
    bus = ExecutionQueueBus.from_workflow_bus(
        WorkflowBus(topics=_DEV_TOPICS)
    )

    bus.publish_request(
        ExecutionQueueRequest(
            request_id="req-2",
            workflow_id="wf-2",
            step_id="step-2",
            executor_id="executor://ml-primary",
            executor_kind="ml-executor",
            capability="ml.embed",
            policy={},
            constraints={},
            payload={"module": "modelado.executors.loaders"},
            transport={"kind": "redpanda", "request_topic": "execution.requests"},
        )
    )

    assert bus.messages == [
        (
            "dev.execution.requests",
            {
                "request_id": "req-2",
                "workflow_id": "wf-2",
                "step_id": "step-2",
                "executor_id": "executor://ml-primary",
                "executor_kind": "ml-executor",
                "capability": "ml.embed",
                "policy": {},
                "constraints": {},
                "payload": {"module": "modelado.executors.loaders"},
                "transport": {"kind": "redpanda", "request_topic": "execution.requests"},
            },
        ),
        (
            "dev.workflow.events",
            ExecutionQueued(
                request_id="req-2",
                workflow_id="wf-2",
                step_id="step-2",
                executor_id="executor://ml-primary",
                executor_kind="ml-executor",
                capability="ml.embed",
            ).model_dump(mode="json"),
        )
    ]


def test_attach_execution_queue_bus_returns_operator_env_with_bridged_bus() -> None:
    env = OperatorEnv(
        seed=1,
        renderer_version="test",
        policy="strict",
        env_scope=EnvironmentScope(ref="refs/heads/run/env-1"),
        slots={"current_marking": {"id": "marking-1"}},
    )

    wired = attach_execution_queue_bus(
        env,
        workflow_bus=WorkflowBus(topics=_DEV_TOPICS),
    )

    assert wired is not env
    assert wired.slots["current_marking"] == {"id": "marking-1"}
    assert isinstance(wired.slots["execution_queue_bus"], ExecutionQueueBus)

    wired.slots["execution_queue_bus"].publish_request(
        ExecutionQueueRequest(
            request_id="req-3",
            workflow_id="wf-3",
            step_id="step-3",
            executor_id="executor://python-primary",
            executor_kind="python-executor",
            capability="python.parse_artifacts",
            policy={},
            constraints={},
            payload={"module": "modelado.executors.loaders"},
            transport={"kind": "redpanda", "request_topic": "execution.requests"},
        )
    )

    assert wired.slots["execution_queue_bus"].messages == [
        (
            "dev.execution.requests",
            {
                "request_id": "req-3",
                "workflow_id": "wf-3",
                "step_id": "step-3",
                "executor_id": "executor://python-primary",
                "executor_kind": "python-executor",
                "capability": "python.parse_artifacts",
                "policy": {},
                "constraints": {},
                "payload": {"module": "modelado.executors.loaders"},
                "transport": {"kind": "redpanda", "request_topic": "execution.requests"},
            },
        ),
        (
            "dev.workflow.events",
            ExecutionQueued(
                request_id="req-3",
                workflow_id="wf-3",
                step_id="step-3",
                executor_id="executor://python-primary",
                executor_kind="python-executor",
                capability="python.parse_artifacts",
            ).model_dump(mode="json"),
        )
    ]
