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
from interacciones.schemas.execution import ApprovalRequested, ApprovalResolved
from interacciones.schemas.execution import ExecutionCompleted, ExecutionFailed, ExecutionProgress, ExecutionQueued
from interacciones.schemas.executors import ExecutorDeclaration


def _orchestrator() -> tuple[WorkflowOrchestrator, InMemoryWorkflowStateStore, InMemoryWorkflowTraceStore]:
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
    return orchestrator, store, trace_store


def test_workflow_execution_event_handlers_route_progress_payloads() -> None:
    orchestrator, store, _ = _orchestrator()
    handlers = WorkflowExecutionEventHandlers(orchestrator)

    handlers.handle_progress(
        {
            "request_id": "req-1",
            "workflow_id": "wf-1",
            "step_id": "dispatch-parse",
            "executor_id": "executor://python-primary",
            "status": "running",
            "progress": 0.5,
            "message": "halfway",
        }
    )

    assert store.get("wf-1") == {
        "workflow_id": "wf-1",
        "current_step": "dispatch-parse",
        "status": "running",
        "executor_id": "executor://python-primary",
        "progress": 0.5,
        "message": "halfway",
        "stdout_lines": [],
        "stderr_lines": [],
    }


def test_workflow_execution_event_handlers_route_queued_payloads() -> None:
    orchestrator, store, _ = _orchestrator()
    handlers = WorkflowExecutionEventHandlers(orchestrator)

    handlers.handle_queued(
        ExecutionQueued(
            request_id="req-queued-1",
            workflow_id="wf-queued-1",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            executor_kind="python-executor",
            capability="python.parse_artifacts",
        ).model_dump(mode="json")
    )

    assert store.get("wf-queued-1") == {
        "workflow_id": "wf-queued-1",
        "current_step": "dispatch-parse",
        "status": "queued",
        "executor_id": "executor://python-primary",
        "request_id": "req-queued-1",
        "capability": "python.parse_artifacts",
        "executor_kind": "python-executor",
    }


def test_workflow_execution_event_handlers_route_completion_and_failure_payloads() -> None:
    orchestrator, store, _ = _orchestrator()
    handlers = WorkflowExecutionEventHandlers(orchestrator)

    handlers.handle_completed(
        ExecutionCompleted(
            request_id="req-1",
            workflow_id="wf-1",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            result={"documents": 1},
            artifacts=["artifact://parsed"],
        ).model_dump(mode="json")
    )

    assert store.get("wf-1") == {
        "workflow_id": "wf-1",
        "current_step": "dispatch-parse",
        "status": "completed",
        "executor_id": "executor://python-primary",
        "result": {"documents": 1},
        "artifacts": ["artifact://parsed"],
        "stdout_lines": [],
        "stderr_lines": [],
    }


def test_workflow_execution_event_handlers_route_retryable_failure_into_retry_state() -> None:
    orchestrator, store, _ = _orchestrator()
    handlers = WorkflowExecutionEventHandlers(orchestrator)

    handlers.handle_failed(
        ExecutionFailed(
            request_id="req-1",
            workflow_id="wf-3",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            error_code="executor_timeout",
            error_message="timed out",
            retryable=True,
            details={
                "retry_count": 1,
                "max_retries": 3,
                "deadline_at": "2026-03-08T12:00:00Z",
            },
        ).model_dump(mode="json")
    )

    assert store.get("wf-3") == {
        "workflow_id": "wf-3",
        "current_step": "dispatch-parse",
        "status": "retry_scheduled",
        "retry_count": 1,
        "max_retries": 3,
        "deadline_at": "2026-03-08T12:00:00Z",
    }


def test_workflow_execution_event_handlers_route_approval_payloads() -> None:
    orchestrator, store, _ = _orchestrator()
    handlers = WorkflowExecutionEventHandlers(orchestrator)

    handlers.handle_approval_requested(
        ApprovalRequested(
            approval_id="approval-1",
            workflow_id="wf-2",
            step_id="await-approval",
            requested_by="dispatcher",
            summary="Need approval",
            details={"reason": "sensitive"},
        ).model_dump(mode="json")
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

    handlers.handle_approval_resolved(
        ApprovalResolved(
            approval_id="approval-1",
            workflow_id="wf-2",
            step_id="await-approval",
            resolved_by="reviewer",
            approved=False,
            comment="needs changes",
        ).model_dump(mode="json")
    )

    assert store.get("wf-2") == {
        "workflow_id": "wf-2",
        "current_step": "await-approval",
        "status": "rejected",
        "approval_id": "approval-1",
        "resolved_by": "reviewer",
        "approved": False,
        "comment": "needs changes",
    }


def test_workflow_execution_event_handlers_route_parse_review_elicitation_payload() -> None:
    orchestrator, store, _ = _orchestrator()
    handlers = WorkflowExecutionEventHandlers(orchestrator)

    handlers.handle_approval_requested(
        ApprovalRequested(
            approval_id="approval-parse-1",
            workflow_id="wf-parse-1",
            step_id="map.conceptual.lift.surface_fragments",
            requested_by="executor://agent-env-primary",
            summary="Parse review rejected semantic map for human approval",
            details={
                "kind": "parse_review_elicitation",
                "approval_mode": "human_required",
                "judgment": {"decision": "reject", "confidence": 0.4},
            },
        ).model_dump(mode="json")
    )

    assert store.get("wf-parse-1") == {
        "workflow_id": "wf-parse-1",
        "current_step": "map.conceptual.lift.surface_fragments",
        "status": "pending_approval",
        "approval_id": "approval-parse-1",
        "requested_by": "executor://agent-env-primary",
        "summary": "Parse review rejected semantic map for human approval",
        "details": {
            "kind": "parse_review_elicitation",
            "approval_mode": "human_required",
            "judgment": {"decision": "reject", "confidence": 0.4},
        },
    }


def test_workflow_execution_event_handlers_emit_trace_events_via_orchestrator() -> None:
    orchestrator, _, trace_store = _orchestrator()
    handlers = WorkflowExecutionEventHandlers(orchestrator)

    handlers.handle_failed(
        ExecutionFailed(
            request_id="req-trace-handlers-1",
            workflow_id="wf-trace-handlers-1",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            error_code="executor_timeout",
            error_message="timed out",
            retryable=False,
        ).model_dump(mode="json")
    )
    handlers.handle_approval_requested(
        ApprovalRequested(
            approval_id="approval-trace-handlers-1",
            workflow_id="wf-trace-handlers-1",
            step_id="await-approval",
            requested_by="dispatcher",
            summary="Need approval",
            details={"reason": "sensitive"},
        ).model_dump(mode="json")
    )

    events = trace_store.list_events(workflow_id="wf-trace-handlers-1", run_id="wf-trace-handlers-1")

    assert [event["event_type"] for event in events] == ["execution.failed", "approval.requested"]


def test_workflow_execution_event_handlers_preserve_petri_trace_fields_from_execution_payloads() -> None:
    orchestrator, _, trace_store = _orchestrator()
    handlers = WorkflowExecutionEventHandlers(orchestrator)

    handlers.handle_progress(
        {
            "request_id": "req-trace-handlers-2",
            "workflow_id": "wf-trace-handlers-2",
            "step_id": "dispatch-parse",
            "executor_id": "executor://python-primary",
            "status": "running",
            "progress": 0.5,
            "message": "halfway",
            "details": {
                "transition_id": "transition:dispatch-parse",
                "marking_before_ref": "marking://before-handlers-2",
                "marking_after_ref": "marking://after-handlers-2",
                "enabled_transition_ids": ["transition:review"],
            },
        }
    )
    handlers.handle_failed(
        {
            "request_id": "req-trace-handlers-2",
            "workflow_id": "wf-trace-handlers-2",
            "step_id": "dispatch-parse",
            "executor_id": "executor://python-primary",
            "error_code": "executor_timeout",
            "error_message": "timed out",
            "retryable": False,
            "details": {
                "transition_id": "transition:dispatch-parse",
                "marking_before_ref": "marking://before-handlers-2b",
                "marking_after_ref": "marking://after-handlers-2b",
                "enabled_transition_ids": ["transition:retry", "transition:fail"],
            },
        }
    )

    events = trace_store.list_events(workflow_id="wf-trace-handlers-2", run_id="wf-trace-handlers-2")

    assert events[0]["transition_id"] == "transition:dispatch-parse"
    assert events[0]["marking_before_ref"] == "marking://before-handlers-2"
    assert events[0]["marking_after_ref"] == "marking://after-handlers-2"
    assert events[0]["enabled_transition_ids"] == ["transition:review"]
    assert events[1]["transition_id"] == "transition:dispatch-parse"
    assert events[1]["marking_before_ref"] == "marking://before-handlers-2b"
    assert events[1]["marking_after_ref"] == "marking://after-handlers-2b"
    assert events[1]["enabled_transition_ids"] == ["transition:retry", "transition:fail"]
