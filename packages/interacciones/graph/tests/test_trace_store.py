from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "packages/interacciones/graph/src"))

from interacciones.graph.workflow.trace_store import InMemoryWorkflowTraceStore


def test_in_memory_trace_store_appends_and_reads_hot_trace_events() -> None:
    store = InMemoryWorkflowTraceStore()

    store.append(
        {
            "trace_id": "trace-1",
            "workflow_id": "wf-1",
            "run_id": "run-1",
            "step_id": "dispatch-parse",
            "event_type": "execution.dispatched",
            "occurred_at": "2026-03-08T12:00:00Z",
            "transition_id": "transition-1",
            "marking_before_ref": "marking://before-1",
            "marking_after_ref": "marking://after-1",
            "enabled_transition_ids": ["transition-2", "transition-3"],
            "request_id": "req-1",
            "executor_id": "executor://python-primary",
            "payload": {"input": "hello"},
        }
    )
    store.append(
        {
            "trace_id": "trace-2",
            "workflow_id": "wf-1",
            "run_id": "run-1",
            "step_id": "dispatch-parse",
            "event_type": "execution.progress",
            "occurred_at": "2026-03-08T12:00:02Z",
            "request_id": "req-1",
            "executor_id": "executor://python-primary",
            "payload": {"progress": 0.5},
        }
    )

    events = store.list_events(workflow_id="wf-1", run_id="run-1")

    assert events == [
        {
            "trace_id": "trace-1",
            "workflow_id": "wf-1",
            "run_id": "run-1",
            "step_id": "dispatch-parse",
            "event_type": "execution.dispatched",
            "occurred_at": "2026-03-08T12:00:00Z",
            "transition_id": "transition-1",
            "marking_before_ref": "marking://before-1",
            "marking_after_ref": "marking://after-1",
            "enabled_transition_ids": ["transition-2", "transition-3"],
            "request_id": "req-1",
            "executor_id": "executor://python-primary",
            "payload": {"input": "hello"},
        },
        {
            "trace_id": "trace-2",
            "workflow_id": "wf-1",
            "run_id": "run-1",
            "step_id": "dispatch-parse",
            "event_type": "execution.progress",
            "occurred_at": "2026-03-08T12:00:02Z",
            "request_id": "req-1",
            "executor_id": "executor://python-primary",
            "payload": {"progress": 0.5},
        },
    ]


def test_in_memory_trace_store_filters_by_workflow_and_run() -> None:
    store = InMemoryWorkflowTraceStore()

    store.append(
        {
            "trace_id": "trace-1",
            "workflow_id": "wf-1",
            "run_id": "run-1",
            "step_id": "dispatch-parse",
            "event_type": "execution.dispatched",
            "occurred_at": "2026-03-08T12:00:00Z",
            "payload": {},
        }
    )
    store.append(
        {
            "trace_id": "trace-2",
            "workflow_id": "wf-1",
            "run_id": "run-2",
            "step_id": "dispatch-parse",
            "event_type": "execution.dispatched",
            "occurred_at": "2026-03-08T12:00:01Z",
            "payload": {},
        }
    )
    store.append(
        {
            "trace_id": "trace-3",
            "workflow_id": "wf-2",
            "run_id": "run-1",
            "step_id": "dispatch-parse",
            "event_type": "execution.dispatched",
            "occurred_at": "2026-03-08T12:00:02Z",
            "payload": {},
        }
    )

    assert [event["trace_id"] for event in store.list_events(workflow_id="wf-1")] == ["trace-1", "trace-2"]
    assert [event["trace_id"] for event in store.list_events(workflow_id="wf-1", run_id="run-2")] == ["trace-2"]
