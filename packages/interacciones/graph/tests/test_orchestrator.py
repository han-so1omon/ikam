from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "packages/interacciones/graph/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))

from interacciones.graph.workflow.orchestrator import WorkflowOrchestrator
from interacciones.graph.workflow.scheduler import WorkflowScheduler
from interacciones.graph.workflow.state_store import InMemoryWorkflowStateStore
from interacciones.graph.workflow.trace_promotion_sink import IkamTracePromotionSink, InMemoryTracePromotionSink
from interacciones.graph.workflow.trace_store import InMemoryWorkflowTraceStore
from interacciones.schemas import TracePersistenceMode, TracePersistencePolicy
from interacciones.schemas.execution import ApprovalRequested, ApprovalResolved, ExecutionCompleted, ExecutionFailed, ExecutionProgress, ExecutionQueued, ExecutionRequest
from interacciones.schemas.executors import ExecutorDeclaration


def _declarations() -> list[ExecutorDeclaration]:
    return [
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
    ]


def test_in_memory_workflow_state_store_tracks_latest_step_status() -> None:
    store = InMemoryWorkflowStateStore()

    store.record_dispatch(
        workflow_id="wf-1",
        step_id="dispatch-parse",
        executor_id="executor://python-primary",
        request_id="req-1",
        capability="python.parse_artifacts",
        policy={"cost_tier": "standard"},
        constraints={},
        payload={"input": "hello"},
    )

    assert store.get("wf-1") == {
        "workflow_id": "wf-1",
        "current_step": "dispatch-parse",
        "status": "dispatched",
        "executor_id": "executor://python-primary",
        "request_id": "req-1",
        "capability": "python.parse_artifacts",
        "policy": {"cost_tier": "standard"},
        "constraints": {},
        "payload": {"input": "hello"},
    }


def test_workflow_orchestrator_dispatches_request_and_records_state() -> None:
    store = InMemoryWorkflowStateStore()
    orchestrator = WorkflowOrchestrator(_declarations(), state_store=store)
    request = ExecutionRequest(
        request_id="req-1",
        workflow_id="wf-1",
        step_id="dispatch-parse",
        capability="python.parse_artifacts",
        policy={"cost_tier": "standard"},
        constraints={},
        payload={"input": "hello"},
    )

    published = orchestrator.dispatch_execution(request)

    assert published.topic == "execution.requests"
    assert published.payload["executor_id"] == "executor://python-primary"
    assert store.get("wf-1") == {
        "workflow_id": "wf-1",
        "current_step": "dispatch-parse",
        "status": "dispatched",
        "executor_id": "executor://python-primary",
        "request_id": "req-1",
        "capability": "python.parse_artifacts",
        "policy": {"cost_tier": "standard"},
        "constraints": {},
        "payload": {"input": "hello"},
    }


def test_workflow_orchestrator_emits_trace_event_on_dispatch() -> None:
    store = InMemoryWorkflowStateStore()
    trace_store = InMemoryWorkflowTraceStore()
    orchestrator = WorkflowOrchestrator(_declarations(), state_store=store, trace_store=trace_store)

    orchestrator.dispatch_execution(
        ExecutionRequest(
            request_id="req-trace-1",
            workflow_id="wf-trace-1",
            step_id="dispatch-parse",
            capability="python.parse_artifacts",
            policy={"cost_tier": "standard"},
            constraints={},
            payload={"input": "hello"},
        )
    )

    events = trace_store.list_events(workflow_id="wf-trace-1", run_id="wf-trace-1")

    assert len(events) == 1
    assert events[0]["event_type"] == "execution.dispatched"
    assert events[0]["step_id"] == "dispatch-parse"
    assert events[0]["request_id"] == "req-trace-1"
    assert events[0]["executor_id"] == "executor://python-primary"
    assert events[0]["payload"] == {"input": "hello"}
    assert events[0]["trace_id"]
    assert events[0]["occurred_at"]


def test_workflow_orchestrator_projects_petri_trace_fields_from_dispatch_payload() -> None:
    store = InMemoryWorkflowStateStore()
    trace_store = InMemoryWorkflowTraceStore()
    orchestrator = WorkflowOrchestrator(_declarations(), state_store=store, trace_store=trace_store)

    orchestrator.dispatch_execution(
        ExecutionRequest(
            request_id="req-trace-petri-1",
            workflow_id="wf-trace-petri-1",
            step_id="dispatch-parse",
            capability="python.parse_artifacts",
            policy={"cost_tier": "standard"},
            constraints={},
            payload={
                "input": "hello",
                "_trace": {
                    "transition_id": "transition:dispatch-parse",
                    "marking_before_ref": "marking://before-1",
                    "marking_after_ref": "marking://after-1",
                    "enabled_transition_ids": ["transition:review", "transition:complete"],
                },
            },
        )
    )

    [event] = trace_store.list_events(workflow_id="wf-trace-petri-1", run_id="wf-trace-petri-1")

    assert event["transition_id"] == "transition:dispatch-parse"
    assert event["marking_before_ref"] == "marking://before-1"
    assert event["marking_after_ref"] == "marking://after-1"
    assert event["enabled_transition_ids"] == ["transition:review", "transition:complete"]


def test_workflow_orchestrator_records_execution_progress() -> None:
    store = InMemoryWorkflowStateStore()
    orchestrator = WorkflowOrchestrator(_declarations(), state_store=store)

    orchestrator.handle_execution_progress(
        ExecutionProgress(
            request_id="req-1",
            workflow_id="wf-1",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            status="running",
            progress=0.5,
            message="halfway",
        )
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


def test_workflow_orchestrator_records_execution_queued_and_reconciles_later_progress() -> None:
    store = InMemoryWorkflowStateStore()
    orchestrator = WorkflowOrchestrator(_declarations(), state_store=store)

    orchestrator.handle_execution_event(
        ExecutionQueued(
            request_id="req-queued-1",
            workflow_id="wf-queued-1",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            executor_kind="python-executor",
            capability="python.parse_artifacts",
        )
    )
    orchestrator.handle_execution_progress(
        ExecutionProgress(
            request_id="req-queued-1",
            workflow_id="wf-queued-1",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            status="running",
            progress=0.5,
            message="halfway",
        )
    )

    assert store.get("wf-queued-1") == {
        "workflow_id": "wf-queued-1",
        "current_step": "dispatch-parse",
        "status": "running",
        "executor_id": "executor://python-primary",
        "request_id": "req-queued-1",
        "capability": "python.parse_artifacts",
        "executor_kind": "python-executor",
        "progress": 0.5,
        "message": "halfway",
        "stdout_lines": [],
        "stderr_lines": [],
    }


def test_workflow_orchestrator_records_execution_queued_and_reconciles_completion() -> None:
    store = InMemoryWorkflowStateStore()
    orchestrator = WorkflowOrchestrator(_declarations(), state_store=store)

    orchestrator.handle_execution_event(
        ExecutionQueued(
            request_id="req-queued-2",
            workflow_id="wf-queued-2",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            executor_kind="python-executor",
            capability="python.parse_artifacts",
        )
    )
    orchestrator.handle_execution_completed(
        ExecutionCompleted(
            request_id="req-queued-2",
            workflow_id="wf-queued-2",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            result={"documents": 1},
            artifacts=["artifact://parsed"],
        )
    )

    assert store.get("wf-queued-2") == {
        "workflow_id": "wf-queued-2",
        "current_step": "dispatch-parse",
        "status": "completed",
        "executor_id": "executor://python-primary",
        "request_id": "req-queued-2",
        "capability": "python.parse_artifacts",
        "executor_kind": "python-executor",
        "result": {"documents": 1},
        "artifacts": ["artifact://parsed"],
        "stdout_lines": [],
        "stderr_lines": [],
    }


def test_workflow_orchestrator_emits_trace_event_on_execution_progress() -> None:
    store = InMemoryWorkflowStateStore()
    trace_store = InMemoryWorkflowTraceStore()
    orchestrator = WorkflowOrchestrator(_declarations(), state_store=store, trace_store=trace_store)

    orchestrator.handle_execution_progress(
        ExecutionProgress(
            request_id="req-trace-2",
            workflow_id="wf-trace-2",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            status="running",
            progress=0.5,
            message="halfway",
        )
    )

    events = trace_store.list_events(workflow_id="wf-trace-2", run_id="wf-trace-2")

    assert len(events) == 1
    assert events[0]["event_type"] == "execution.progress"
    assert events[0]["step_id"] == "dispatch-parse"
    assert events[0]["request_id"] == "req-trace-2"
    assert events[0]["executor_id"] == "executor://python-primary"
    assert events[0]["payload"] == {
        "status": "running",
        "progress": 0.5,
        "message": "halfway",
        "stdout_lines": [],
        "stderr_lines": [],
    }
    assert events[0]["trace_id"]
    assert events[0]["occurred_at"]


def test_workflow_orchestrator_projects_petri_trace_fields_from_execution_progress_details() -> None:
    store = InMemoryWorkflowStateStore()
    trace_store = InMemoryWorkflowTraceStore()
    orchestrator = WorkflowOrchestrator(_declarations(), state_store=store, trace_store=trace_store)

    orchestrator.handle_execution_progress(
        ExecutionProgress(
            request_id="req-trace-petri-2",
            workflow_id="wf-trace-petri-2",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            status="running",
            progress=0.5,
            message="halfway",
            details={
                "transition_id": "transition:dispatch-parse",
                "marking_before_ref": "marking://before-2",
                "marking_after_ref": "marking://after-2",
                "enabled_transition_ids": ["transition:review"],
            },
        )
    )

    [event] = trace_store.list_events(workflow_id="wf-trace-petri-2", run_id="wf-trace-petri-2")

    assert event["transition_id"] == "transition:dispatch-parse"
    assert event["marking_before_ref"] == "marking://before-2"
    assert event["marking_after_ref"] == "marking://after-2"
    assert event["enabled_transition_ids"] == ["transition:review"]


def test_execution_progress_appends_log_chunks_to_same_step_attempt() -> None:
    store = InMemoryWorkflowStateStore()
    trace_store = InMemoryWorkflowTraceStore()
    orchestrator = WorkflowOrchestrator(_declarations(), state_store=store, trace_store=trace_store)

    progress_started = ExecutionProgress.model_construct(
        request_id="req-log-1",
        workflow_id="wf-log-1",
        step_id="dispatch-parse",
        executor_id="executor://python-primary",
        status="running",
        progress=0.1,
        message="started",
        details={},
    )
    object.__setattr__(progress_started, "stdout_lines", ["chunk 1"])
    object.__setattr__(progress_started, "stderr_lines", [])
    orchestrator.handle_execution_progress(
        progress_started
    )

    progress_continued = ExecutionProgress.model_construct(
        request_id="req-log-1",
        workflow_id="wf-log-1",
        step_id="dispatch-parse",
        executor_id="executor://python-primary",
        status="running",
        progress=0.2,
        message="continued",
        details={},
    )
    object.__setattr__(progress_continued, "stdout_lines", ["chunk 2"])
    object.__setattr__(progress_continued, "stderr_lines", [])
    orchestrator.handle_execution_progress(
        progress_continued
    )

    completed = ExecutionCompleted.model_construct(
        request_id="req-log-1",
        workflow_id="wf-log-1",
        step_id="dispatch-parse",
        executor_id="executor://python-primary",
        result={"documents": 1},
        artifacts=["artifact://parsed"],
    )
    object.__setattr__(completed, "stdout_lines", [])
    object.__setattr__(completed, "stderr_lines", ["warn 1"])
    orchestrator.handle_execution_completed(
        completed
    )

    state = store.get("wf-log-1")

    assert state is not None
    assert state["stdout_lines"] == ["chunk 1", "chunk 2"]
    assert state["stderr_lines"] == ["warn 1"]
    events = trace_store.list_events(workflow_id="wf-log-1", run_id="wf-log-1")
    assert [event["payload"].get("stdout_lines", []) for event in events] == [["chunk 1"], ["chunk 2"], []]
    assert [event["payload"].get("stderr_lines", []) for event in events] == [[], [], ["warn 1"]]


def test_workflow_orchestrator_records_execution_completion() -> None:
    store = InMemoryWorkflowStateStore()
    orchestrator = WorkflowOrchestrator(_declarations(), state_store=store)

    orchestrator.handle_execution_completed(
        ExecutionCompleted(
            request_id="req-1",
            workflow_id="wf-1",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            result={"documents": 1},
            artifacts=["artifact://parsed"],
        )
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


def test_workflow_orchestrator_records_execution_failure() -> None:
    store = InMemoryWorkflowStateStore()
    orchestrator = WorkflowOrchestrator(_declarations(), state_store=store)

    orchestrator.handle_execution_failed(
        ExecutionFailed(
            request_id="req-1",
            workflow_id="wf-1",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            error_code="executor_timeout",
            error_message="timed out",
            retryable=True,
        )
    )

    assert store.get("wf-1") == {
        "workflow_id": "wf-1",
        "current_step": "dispatch-parse",
        "status": "failed",
        "executor_id": "executor://python-primary",
        "error_code": "executor_timeout",
        "error_message": "timed out",
        "retryable": True,
        "stdout_lines": [],
        "stderr_lines": [],
    }


def test_workflow_orchestrator_emits_trace_event_on_execution_failure() -> None:
    store = InMemoryWorkflowStateStore()
    trace_store = InMemoryWorkflowTraceStore()
    orchestrator = WorkflowOrchestrator(_declarations(), state_store=store, trace_store=trace_store)

    orchestrator.handle_execution_failed(
        ExecutionFailed(
            request_id="req-trace-3",
            workflow_id="wf-trace-3",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            error_code="executor_timeout",
            error_message="timed out",
            retryable=True,
        )
    )

    events = trace_store.list_events(workflow_id="wf-trace-3", run_id="wf-trace-3")

    assert len(events) == 1
    assert events[0]["event_type"] == "execution.failed"
    assert events[0]["step_id"] == "dispatch-parse"
    assert events[0]["request_id"] == "req-trace-3"
    assert events[0]["executor_id"] == "executor://python-primary"
    assert events[0]["payload"] == {
        "error_code": "executor_timeout",
        "error_message": "timed out",
        "retryable": True,
        "stdout_lines": [],
        "stderr_lines": [],
    }
    assert events[0]["trace_id"]
    assert events[0]["occurred_at"]


def test_workflow_orchestrator_projects_petri_trace_fields_from_execution_failure_details() -> None:
    store = InMemoryWorkflowStateStore()
    trace_store = InMemoryWorkflowTraceStore()
    orchestrator = WorkflowOrchestrator(_declarations(), state_store=store, trace_store=trace_store)

    orchestrator.handle_execution_failed(
        ExecutionFailed(
            request_id="req-trace-petri-3",
            workflow_id="wf-trace-petri-3",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            error_code="executor_timeout",
            error_message="timed out",
            retryable=False,
            details={
                "transition_id": "transition:dispatch-parse",
                "marking_before_ref": "marking://before-3",
                "marking_after_ref": "marking://after-3",
                "enabled_transition_ids": ["transition:retry", "transition:fail"],
            },
        )
    )

    [event] = trace_store.list_events(workflow_id="wf-trace-petri-3", run_id="wf-trace-petri-3")

    assert event["transition_id"] == "transition:dispatch-parse"
    assert event["marking_before_ref"] == "marking://before-3"
    assert event["marking_after_ref"] == "marking://after-3"
    assert event["enabled_transition_ids"] == ["transition:retry", "transition:fail"]


def test_workflow_orchestrator_collects_promotion_plan_when_failure_policy_triggers() -> None:
    store = InMemoryWorkflowStateStore()
    trace_store = InMemoryWorkflowTraceStore()
    orchestrator = WorkflowOrchestrator(
        _declarations(),
        state_store=store,
        trace_store=trace_store,
        trace_policy=TracePersistencePolicy(mode=TracePersistenceMode.ON_FAILURE),
    )

    orchestrator.dispatch_execution(
        ExecutionRequest(
            request_id="req-promote-1",
            workflow_id="wf-promote-1",
            step_id="dispatch-parse",
            capability="python.parse_artifacts",
            policy={"cost_tier": "standard"},
            constraints={},
            payload={"input": "hello", "ref": "refs/heads/run/run-promote-1"},
        )
    )
    orchestrator.handle_execution_failed(
        ExecutionFailed(
            request_id="req-promote-1",
            workflow_id="wf-promote-1",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            error_code="executor_timeout",
            error_message="timed out",
            retryable=False,
        )
    )

    assert orchestrator.get_pending_trace_promotions() == [
        {
            "workflow_id": "wf-promote-1",
            "run_id": "wf-promote-1",
            "target_ref": "refs/heads/run/run-promote-1",
            "policy_mode": "on_failure",
            "trigger_event_type": "execution.failed",
            "trace_ids": [
                trace_store.list_events(workflow_id="wf-promote-1", run_id="wf-promote-1")[0]["trace_id"],
                trace_store.list_events(workflow_id="wf-promote-1", run_id="wf-promote-1")[1]["trace_id"],
            ],
            "event_count": 2,
        }
    ]


def test_workflow_orchestrator_tracks_run_ref_across_trace_events() -> None:
    store = InMemoryWorkflowStateStore()
    trace_store = InMemoryWorkflowTraceStore()
    orchestrator = WorkflowOrchestrator(_declarations(), state_store=store, trace_store=trace_store)

    orchestrator.dispatch_execution(
        ExecutionRequest(
            request_id="req-ref-1",
            workflow_id="wf-ref-1",
            step_id="dispatch-parse",
            capability="python.parse_artifacts",
            policy={"cost_tier": "standard"},
            constraints={},
            payload={"input": "hello", "ref": "refs/heads/run/run-ref-1"},
        )
    )
    orchestrator.handle_execution_failed(
        ExecutionFailed(
            request_id="req-ref-1",
            workflow_id="wf-ref-1",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            error_code="executor_timeout",
            error_message="timed out",
            retryable=False,
        )
    )

    events = trace_store.list_events(workflow_id="wf-ref-1", run_id="wf-ref-1")

    assert [event["ref"] for event in events] == ["refs/heads/run/run-ref-1", "refs/heads/run/run-ref-1"]


def test_workflow_orchestrator_flushes_pending_trace_promotions_into_sink() -> None:
    store = InMemoryWorkflowStateStore()
    trace_store = InMemoryWorkflowTraceStore()
    sink = InMemoryTracePromotionSink()
    orchestrator = WorkflowOrchestrator(
        _declarations(),
        state_store=store,
        trace_store=trace_store,
        trace_policy=TracePersistencePolicy(mode=TracePersistenceMode.ON_FAILURE),
        trace_promotion_sink=sink,
    )

    orchestrator.dispatch_execution(
        ExecutionRequest(
            request_id="req-promote-2",
            workflow_id="wf-promote-2",
            step_id="dispatch-parse",
            capability="python.parse_artifacts",
            policy={"cost_tier": "standard"},
            constraints={},
            payload={"input": "hello"},
        )
    )
    orchestrator.handle_execution_failed(
        ExecutionFailed(
            request_id="req-promote-2",
            workflow_id="wf-promote-2",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            error_code="executor_timeout",
            error_message="timed out",
            retryable=False,
        )
    )

    flushed = orchestrator.flush_trace_promotions()

    assert len(flushed) == 1
    assert flushed == sink.plans
    assert orchestrator.get_pending_trace_promotions() == []


def test_workflow_orchestrator_flushes_pending_trace_promotions_into_ikam_sink() -> None:
    store = InMemoryWorkflowStateStore()
    trace_store = InMemoryWorkflowTraceStore()
    sink = IkamTracePromotionSink(writer=lambda plan: "fragment://trace-bundle-2")
    orchestrator = WorkflowOrchestrator(
        _declarations(),
        state_store=store,
        trace_store=trace_store,
        trace_policy=TracePersistencePolicy(mode=TracePersistenceMode.ON_FAILURE),
        trace_promotion_sink=sink,
    )

    orchestrator.dispatch_execution(
        ExecutionRequest(
            request_id="req-promote-3",
            workflow_id="wf-promote-3",
            step_id="dispatch-parse",
            capability="python.parse_artifacts",
            policy={"cost_tier": "standard"},
            constraints={},
            payload={"input": "hello"},
        )
    )
    orchestrator.handle_execution_failed(
        ExecutionFailed(
            request_id="req-promote-3",
            workflow_id="wf-promote-3",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            error_code="executor_timeout",
            error_message="timed out",
            retryable=False,
        )
    )

    flushed = orchestrator.flush_trace_promotions()

    assert flushed == [
        {
            "workflow_id": "wf-promote-3",
            "run_id": "wf-promote-3",
            "target_ref": None,
            "policy_mode": "on_failure",
            "trigger_event_type": "execution.failed",
            "trace_ids": [
                trace_store.list_events(workflow_id="wf-promote-3", run_id="wf-promote-3")[0]["trace_id"],
                trace_store.list_events(workflow_id="wf-promote-3", run_id="wf-promote-3")[1]["trace_id"],
            ],
            "event_count": 2,
            "committed_trace_fragment_id": "fragment://trace-bundle-2",
        }
    ]
    assert sink.records == flushed


def test_workflow_orchestrator_queues_final_trace_promotion_for_final_only_policy() -> None:
    orchestrator = WorkflowOrchestrator(
        _declarations(),
        state_store=InMemoryWorkflowStateStore(),
        trace_store=InMemoryWorkflowTraceStore(),
        trace_policy=TracePersistencePolicy(mode=TracePersistenceMode.FINAL_ONLY),
    )

    orchestrator.dispatch_execution(
        ExecutionRequest(
            request_id="req-promote-final-1",
            workflow_id="wf-promote-final-1",
            step_id="dispatch-parse",
            capability="python.parse_artifacts",
            policy={"cost_tier": "standard"},
            constraints={},
            payload={"input": "hello"},
        )
    )
    orchestrator.handle_execution_completed(
        ExecutionCompleted(
            request_id="req-promote-final-1",
            workflow_id="wf-promote-final-1",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            result={"documents": 1},
            artifacts=["artifact://parsed"],
        )
    )

    flushed = orchestrator.flush_trace_promotions()

    assert flushed == [
        {
            "workflow_id": "wf-promote-final-1",
            "run_id": "wf-promote-final-1",
            "target_ref": None,
            "policy_mode": "final_only",
            "trigger_event_type": "execution.completed",
            "trace_ids": [
                trace["trace_id"]
                for trace in orchestrator._trace_store.list_events(
                    workflow_id="wf-promote-final-1",
                    run_id="wf-promote-final-1",
                )
            ],
            "event_count": 2,
        }
    ]


def test_workflow_orchestrator_turns_retryable_failure_into_retry_metadata_when_details_present() -> None:
    store = InMemoryWorkflowStateStore()
    orchestrator = WorkflowOrchestrator(_declarations(), state_store=store)

    orchestrator.handle_execution_failed(
        ExecutionFailed(
            request_id="req-1",
            workflow_id="wf-1",
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
        )
    )

    assert store.get("wf-1") == {
        "workflow_id": "wf-1",
        "current_step": "dispatch-parse",
        "status": "retry_scheduled",
        "retry_count": 1,
        "max_retries": 3,
        "deadline_at": "2026-03-08T12:00:00Z",
    }


def test_workflow_orchestrator_handles_execution_events_through_single_entrypoint() -> None:
    store = InMemoryWorkflowStateStore()
    orchestrator = WorkflowOrchestrator(_declarations(), state_store=store)

    orchestrator.handle_execution_event(
        ExecutionProgress(
            request_id="req-1",
            workflow_id="wf-1",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            status="running",
            progress=0.25,
            message="warming",
        )
    )
    assert store.get("wf-1") == {
        "workflow_id": "wf-1",
        "current_step": "dispatch-parse",
        "status": "running",
        "executor_id": "executor://python-primary",
        "progress": 0.25,
        "message": "warming",
        "stdout_lines": [],
        "stderr_lines": [],
    }

    orchestrator.handle_execution_event(
        ExecutionCompleted(
            request_id="req-1",
            workflow_id="wf-1",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            result={"documents": 1},
            artifacts=["artifact://parsed"],
        )
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


def test_in_memory_workflow_state_store_does_not_overwrite_terminal_state_with_progress() -> None:
    store = InMemoryWorkflowStateStore()

    store.record_completion(
        workflow_id="wf-2",
        step_id="dispatch-parse",
        executor_id="executor://python-primary",
        result={"documents": 1},
        artifacts=["artifact://parsed"],
        stdout_lines=[],
        stderr_lines=[],
    )
    store.record_progress(
        workflow_id="wf-2",
        step_id="dispatch-parse",
        executor_id="executor://python-primary",
        status="running",
        progress=0.5,
        message="late-progress",
        stdout_lines=[],
        stderr_lines=[],
    )

    assert store.get("wf-2") == {
        "workflow_id": "wf-2",
        "current_step": "dispatch-parse",
        "status": "completed",
        "executor_id": "executor://python-primary",
        "result": {"documents": 1},
        "artifacts": ["artifact://parsed"],
        "stdout_lines": [],
        "stderr_lines": [],
    }


def test_workflow_orchestrator_records_pending_approval() -> None:
    store = InMemoryWorkflowStateStore()
    orchestrator = WorkflowOrchestrator(_declarations(), state_store=store)

    orchestrator.handle_approval_requested(
        ApprovalRequested(
            approval_id="approval-1",
            workflow_id="wf-3",
            step_id="await-approval",
            requested_by="dispatcher",
            summary="Need approval",
            details={"reason": "sensitive"},
        )
    )

    assert store.get("wf-3") == {
        "workflow_id": "wf-3",
        "current_step": "await-approval",
        "status": "pending_approval",
        "approval_id": "approval-1",
        "requested_by": "dispatcher",
        "summary": "Need approval",
        "details": {"reason": "sensitive"},
    }


def test_workflow_orchestrator_emits_trace_event_on_approval_requested() -> None:
    store = InMemoryWorkflowStateStore()
    trace_store = InMemoryWorkflowTraceStore()
    orchestrator = WorkflowOrchestrator(_declarations(), state_store=store, trace_store=trace_store)

    orchestrator.handle_approval_requested(
        ApprovalRequested(
            approval_id="approval-trace-1",
            workflow_id="wf-trace-4",
            step_id="await-approval",
            requested_by="dispatcher",
            summary="Need approval",
            details={"reason": "sensitive"},
        )
    )

    events = trace_store.list_events(workflow_id="wf-trace-4", run_id="wf-trace-4")

    assert len(events) == 1
    assert events[0]["event_type"] == "approval.requested"
    assert events[0]["step_id"] == "await-approval"
    assert events[0]["approval_id"] == "approval-trace-1"
    assert events[0]["payload"] == {"requested_by": "dispatcher", "summary": "Need approval", "details": {"reason": "sensitive"}}
    assert events[0]["trace_id"]
    assert events[0]["occurred_at"]


def test_workflow_orchestrator_preserves_parse_review_elicitation_details() -> None:
    store = InMemoryWorkflowStateStore()
    orchestrator = WorkflowOrchestrator(_declarations(), state_store=store)

    orchestrator.handle_approval_requested(
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
        )
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


def test_workflow_orchestrator_records_approval_resolution() -> None:
    store = InMemoryWorkflowStateStore()
    orchestrator = WorkflowOrchestrator(_declarations(), state_store=store)

    orchestrator.handle_approval_requested(
        ApprovalRequested(
            approval_id="approval-1",
            workflow_id="wf-3",
            step_id="await-approval",
            requested_by="dispatcher",
            summary="Need approval",
            details={"reason": "sensitive"},
        )
    )
    orchestrator.handle_approval_resolved(
        ApprovalResolved(
            approval_id="approval-1",
            workflow_id="wf-3",
            step_id="await-approval",
            resolved_by="reviewer",
            approved=True,
            comment="looks good",
        )
    )

    assert store.get("wf-3") == {
        "workflow_id": "wf-3",
        "current_step": "await-approval",
        "status": "approved",
        "approval_id": "approval-1",
        "resolved_by": "reviewer",
        "approved": True,
        "comment": "looks good",
    }


def test_workflow_orchestrator_emits_trace_event_on_approval_resolved() -> None:
    store = InMemoryWorkflowStateStore()
    trace_store = InMemoryWorkflowTraceStore()
    orchestrator = WorkflowOrchestrator(_declarations(), state_store=store, trace_store=trace_store)

    orchestrator.handle_approval_resolved(
        ApprovalResolved(
            approval_id="approval-trace-2",
            workflow_id="wf-trace-5",
            step_id="await-approval",
            resolved_by="reviewer",
            approved=True,
            comment="looks good",
        )
    )

    events = trace_store.list_events(workflow_id="wf-trace-5", run_id="wf-trace-5")

    assert len(events) == 1
    assert events[0]["event_type"] == "approval.resolved"
    assert events[0]["step_id"] == "await-approval"
    assert events[0]["approval_id"] == "approval-trace-2"
    assert events[0]["payload"] == {"resolved_by": "reviewer", "approved": True, "comment": "looks good"}
    assert events[0]["trace_id"]
    assert events[0]["occurred_at"]


def test_workflow_orchestrator_records_retry_and_deadline_metadata() -> None:
    store = InMemoryWorkflowStateStore()
    orchestrator = WorkflowOrchestrator(_declarations(), state_store=store)

    orchestrator.record_retry_metadata(
        workflow_id="wf-4",
        step_id="dispatch-parse",
        retry_count=2,
        max_retries=5,
        deadline_at="2026-03-08T12:00:00Z",
    )

    assert store.get("wf-4") == {
        "workflow_id": "wf-4",
        "current_step": "dispatch-parse",
        "status": "retry_scheduled",
        "retry_count": 2,
        "max_retries": 5,
        "deadline_at": "2026-03-08T12:00:00Z",
    }


def test_workflow_orchestrator_redispatches_scheduler_retry_wakeup_when_state_matches() -> None:
    store = InMemoryWorkflowStateStore()
    trace_store = InMemoryWorkflowTraceStore()
    orchestrator = WorkflowOrchestrator(_declarations(), state_store=store, trace_store=trace_store)
    orchestrator.dispatch_execution(
        ExecutionRequest(
            request_id="req-5",
            workflow_id="wf-5",
            step_id="dispatch-parse",
            capability="python.parse_artifacts",
            policy={"cost_tier": "standard"},
            constraints={},
            payload={
                "input": "hello",
                "_trace": {
                    "transition_id": "transition:dispatch-parse",
                    "marking_before_ref": "marking://before-5",
                    "marking_after_ref": "marking://after-5",
                    "enabled_transition_ids": ["transition:retry"],
                },
            },
        )
    )
    orchestrator.record_retry_metadata(
        workflow_id="wf-5",
        step_id="dispatch-parse",
        retry_count=1,
        max_retries=3,
        deadline_at="2026-03-08T12:00:00Z",
    )
    scheduler = WorkflowScheduler(store, lease_owner="scheduler-a")

    [action] = scheduler.tick_due_work(now="2026-03-08T12:00:00Z", lease_duration_seconds=300)

    accepted = orchestrator.handle_scheduler_action(action)

    assert accepted is not None
    assert accepted.topic == "execution.requests"
    assert accepted.payload["request_id"] == "req-5"
    assert accepted.payload["workflow_id"] == "wf-5"
    assert accepted.payload["step_id"] == "dispatch-parse"
    assert accepted.payload["executor_id"] == "executor://python-primary"
    events = trace_store.list_events(workflow_id="wf-5", run_id="wf-5")
    assert [event["event_type"] for event in events] == [
        "execution.dispatched",
        "scheduler.retry_wakeup",
        "execution.dispatched",
    ]
    assert events[1]["lease_owner"] == "scheduler-a"
    assert events[1]["request_id"] == "req-5"
    assert events[1]["transition_id"] == "transition:dispatch-parse"
    assert events[1]["marking_before_ref"] == "marking://before-5"
    assert events[1]["marking_after_ref"] == "marking://after-5"
    assert events[1]["enabled_transition_ids"] == ["transition:retry"]
    assert events[1]["payload"] == {
        "retry_count": 1,
        "max_retries": 3,
        "deadline_at": "2026-03-08T12:00:00Z",
    }
    assert store.get("wf-5") == {
        "workflow_id": "wf-5",
        "current_step": "dispatch-parse",
        "status": "dispatched",
        "executor_id": "executor://python-primary",
        "request_id": "req-5",
        "capability": "python.parse_artifacts",
        "policy": {"cost_tier": "standard"},
        "constraints": {},
        "payload": {
            "input": "hello",
            "_trace": {
                "transition_id": "transition:dispatch-parse",
                "marking_before_ref": "marking://before-5",
                "marking_after_ref": "marking://after-5",
                "enabled_transition_ids": ["transition:retry"],
            },
        },
    }
