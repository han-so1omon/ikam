from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "packages/interacciones/graph/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))

from interacciones.graph.workflow.event_handlers import WorkflowExecutionEventHandlers
from interacciones.graph.workflow.orchestrator import WorkflowOrchestrator
from interacciones.graph.workflow.state_store import InMemoryWorkflowStateStore
from interacciones.graph.workflow.trace_store import InMemoryWorkflowTraceStore
from interacciones.graph.workflow.topic_consumer import WorkflowTopicConsumer
from interacciones.graph.workflow.bus import WorkflowBus
from interacciones.schemas.executors import ExecutorDeclaration
from interacciones.schemas.execution import ExecutionRequest
from interacciones.schemas.topics import OrchestrationTopicNames


_DEV_TOPICS = OrchestrationTopicNames(
    execution_requests="dev.execution.requests",
    execution_progress="dev.execution.progress",
    execution_results="dev.execution.results",
    workflow_events="dev.workflow.events",
    approval_events="dev.approval.events",
    mcp_events="dev.mcp.events",
    acp_events="dev.acp.events",
)


def _consumer() -> tuple[WorkflowTopicConsumer, InMemoryWorkflowStateStore, InMemoryWorkflowTraceStore]:
    store = InMemoryWorkflowStateStore()
    trace_store = InMemoryWorkflowTraceStore()
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
        trace_store=trace_store,
    )
    handlers = WorkflowExecutionEventHandlers(orchestrator)
    return WorkflowTopicConsumer(handlers, topics=_DEV_TOPICS), store, trace_store


def test_workflow_topic_consumer_routes_execution_and_approval_topics() -> None:
    consumer, store, _ = _consumer()

    consumer.consume(
        "dev.workflow.events",
        {
            "request_id": "req-q-1",
            "workflow_id": "wf-1",
            "step_id": "dispatch-parse",
            "executor_id": "executor://python-primary",
            "executor_kind": "python-executor",
            "capability": "python.parse_artifacts",
            "status": "queued",
        },
    )
    assert store.get("wf-1") == {
        "workflow_id": "wf-1",
        "current_step": "dispatch-parse",
        "status": "queued",
        "executor_id": "executor://python-primary",
        "request_id": "req-q-1",
        "capability": "python.parse_artifacts",
        "executor_kind": "python-executor",
    }

    consumer.consume(
        "dev.execution.progress",
        {
            "request_id": "req-q-1",
            "workflow_id": "wf-1",
            "step_id": "dispatch-parse",
            "executor_id": "executor://python-primary",
            "status": "running",
            "progress": 0.25,
            "message": "warming",
        },
    )
    assert store.get("wf-1") == {
        "workflow_id": "wf-1",
        "current_step": "dispatch-parse",
        "status": "running",
        "executor_id": "executor://python-primary",
        "request_id": "req-q-1",
        "capability": "python.parse_artifacts",
        "executor_kind": "python-executor",
        "progress": 0.25,
        "message": "warming",
        "stdout_lines": [],
        "stderr_lines": [],
    }

    consumer.consume(
        "dev.approval.events",
        {
            "approval_id": "approval-1",
            "workflow_id": "wf-2",
            "step_id": "await-approval",
            "requested_by": "dispatcher",
            "summary": "Need approval",
            "details": {"reason": "sensitive"},
        },
    )
    assert store.get("wf-2") == {
        "workflow_id": "wf-2",
        "current_step": "await-approval",
        "status": "pending_approval",
        "approval_id": "approval-1",
        "requested_by": "dispatcher",
        "summary": "Need approval",
        "details": {"reason": "sensitive"},
    }


def test_workflow_topic_consumer_rejects_unknown_topic() -> None:
    consumer, _, _ = _consumer()

    try:
        consumer.consume("dev.unknown.topic", {"x": 1})
    except ValueError as exc:
        assert str(exc) == "unsupported workflow topic: dev.unknown.topic"
    else:
        raise AssertionError("expected unknown topic to be rejected")


def test_workflow_topic_consumer_emits_trace_events_via_runtime_entrypoint() -> None:
    consumer, _, trace_store = _consumer()

    consumer.consume(
        "dev.workflow.events",
        {
            "request_id": "req-trace-consumer-1",
            "workflow_id": "wf-trace-consumer-1",
            "step_id": "dispatch-parse",
            "executor_id": "executor://python-primary",
            "executor_kind": "python-executor",
            "capability": "python.parse_artifacts",
            "status": "queued",
        },
    )
    consumer.consume(
        "dev.execution.results",
        {
            "request_id": "req-trace-consumer-1",
            "workflow_id": "wf-trace-consumer-1",
            "step_id": "dispatch-parse",
            "executor_id": "executor://python-primary",
            "error_code": "executor_timeout",
            "error_message": "timed out",
            "retryable": False,
        },
    )
    consumer.consume(
        "dev.approval.events",
        {
            "approval_id": "approval-trace-consumer-1",
            "workflow_id": "wf-trace-consumer-1",
            "step_id": "await-approval",
            "resolved_by": "reviewer",
            "approved": True,
            "comment": "ok",
        },
    )

    events = trace_store.list_events(workflow_id="wf-trace-consumer-1", run_id="wf-trace-consumer-1")

    assert [event["event_type"] for event in events] == ["execution.queued", "execution.failed", "approval.resolved"]


def test_workflow_topic_consumer_preserves_petri_trace_fields_from_inbound_payloads() -> None:
    consumer, _, trace_store = _consumer()

    consumer.consume(
        "dev.execution.progress",
        {
            "request_id": "req-trace-consumer-2",
            "workflow_id": "wf-trace-consumer-2",
            "step_id": "dispatch-parse",
            "executor_id": "executor://python-primary",
            "status": "running",
            "progress": 0.25,
            "message": "warming",
            "details": {
                "transition_id": "transition:dispatch-parse",
                "marking_before_ref": "marking://before-consumer-2",
                "marking_after_ref": "marking://after-consumer-2",
                "enabled_transition_ids": ["transition:review"],
            },
        },
    )
    consumer.consume(
        "dev.execution.results",
        {
            "request_id": "req-trace-consumer-2",
            "workflow_id": "wf-trace-consumer-2",
            "step_id": "dispatch-parse",
            "executor_id": "executor://python-primary",
            "error_code": "executor_timeout",
            "error_message": "timed out",
            "retryable": False,
            "details": {
                "transition_id": "transition:dispatch-parse",
                "marking_before_ref": "marking://before-consumer-2b",
                "marking_after_ref": "marking://after-consumer-2b",
                "enabled_transition_ids": ["transition:retry", "transition:fail"],
            },
        },
    )

    events = trace_store.list_events(workflow_id="wf-trace-consumer-2", run_id="wf-trace-consumer-2")

    assert events[0]["transition_id"] == "transition:dispatch-parse"
    assert events[0]["marking_before_ref"] == "marking://before-consumer-2"
    assert events[0]["marking_after_ref"] == "marking://after-consumer-2"
    assert events[0]["enabled_transition_ids"] == ["transition:review"]
    assert events[1]["transition_id"] == "transition:dispatch-parse"
    assert events[1]["marking_before_ref"] == "marking://before-consumer-2b"
    assert events[1]["marking_after_ref"] == "marking://after-consumer-2b"
    assert events[1]["enabled_transition_ids"] == ["transition:retry", "transition:fail"]


def test_workflow_bus_and_topic_consumer_drive_end_to_end_queued_lifecycle() -> None:
    consumer, store, trace_store = _consumer()
    bus = WorkflowBus(topics=_DEV_TOPICS)

    queued = bus.publish_workflow_event(
        {
            "request_id": "req-e2e-1",
            "workflow_id": "wf-e2e-1",
            "step_id": "dispatch-parse",
            "executor_id": "executor://python-primary",
            "executor_kind": "python-executor",
            "capability": "python.parse_artifacts",
            "status": "queued",
        }
    )
    progress = bus.publish_execution_progress(
        {
            "request_id": "req-e2e-1",
            "workflow_id": "wf-e2e-1",
            "step_id": "dispatch-parse",
            "executor_id": "executor://python-primary",
            "status": "running",
            "progress": 0.5,
            "message": "halfway",
        }
    )
    completed = bus.publish_execution_result(
        {
            "request_id": "req-e2e-1",
            "workflow_id": "wf-e2e-1",
            "step_id": "dispatch-parse",
            "executor_id": "executor://python-primary",
            "result": {"documents": 1},
            "artifacts": ["artifact://parsed"],
        }
    )

    for published in (queued, progress, completed):
        consumer.consume(published.topic, published.payload)

    assert store.get("wf-e2e-1") == {
        "workflow_id": "wf-e2e-1",
        "current_step": "dispatch-parse",
        "status": "completed",
        "executor_id": "executor://python-primary",
        "request_id": "req-e2e-1",
        "capability": "python.parse_artifacts",
        "executor_kind": "python-executor",
        "result": {"documents": 1},
        "artifacts": ["artifact://parsed"],
        "stdout_lines": [],
        "stderr_lines": [],
    }
    assert [
        event["event_type"]
        for event in trace_store.list_events(workflow_id="wf-e2e-1", run_id="wf-e2e-1")
    ] == ["execution.queued", "execution.progress", "execution.completed"]


def test_workflow_bus_and_topic_consumer_drive_end_to_end_queued_failure_lifecycle() -> None:
    consumer, store, trace_store = _consumer()
    bus = WorkflowBus(topics=_DEV_TOPICS)

    queued = bus.publish_workflow_event(
        {
            "request_id": "req-e2e-fail-1",
            "workflow_id": "wf-e2e-fail-1",
            "step_id": "dispatch-parse",
            "executor_id": "executor://python-primary",
            "executor_kind": "python-executor",
            "capability": "python.parse_artifacts",
            "status": "queued",
        }
    )
    progress = bus.publish_execution_progress(
        {
            "request_id": "req-e2e-fail-1",
            "workflow_id": "wf-e2e-fail-1",
            "step_id": "dispatch-parse",
            "executor_id": "executor://python-primary",
            "status": "running",
            "progress": 0.5,
            "message": "halfway",
        }
    )
    failed = bus.publish_execution_result(
        {
            "request_id": "req-e2e-fail-1",
            "workflow_id": "wf-e2e-fail-1",
            "step_id": "dispatch-parse",
            "executor_id": "executor://python-primary",
            "error_code": "executor_timeout",
            "error_message": "timed out",
            "retryable": False,
        }
    )

    for published in (queued, progress, failed):
        consumer.consume(published.topic, published.payload)

    assert store.get("wf-e2e-fail-1") == {
        "workflow_id": "wf-e2e-fail-1",
        "current_step": "dispatch-parse",
        "status": "failed",
        "executor_id": "executor://python-primary",
        "request_id": "req-e2e-fail-1",
        "capability": "python.parse_artifacts",
        "executor_kind": "python-executor",
        "error_code": "executor_timeout",
        "error_message": "timed out",
        "retryable": False,
        "stdout_lines": [],
        "stderr_lines": [],
    }
    assert [
        event["event_type"]
        for event in trace_store.list_events(workflow_id="wf-e2e-fail-1", run_id="wf-e2e-fail-1")
    ] == ["execution.queued", "execution.progress", "execution.failed"]
