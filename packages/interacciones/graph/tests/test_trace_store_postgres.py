from __future__ import annotations

import os
from contextlib import contextmanager
from pathlib import Path
import sys

import pytest

try:
    import psycopg
except ImportError:  # pragma: no cover - environment dependent
    psycopg = None


ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "packages/interacciones/graph/src"))

from interacciones.graph.workflow.trace_store_postgres import PostgresWorkflowTraceStore


pytestmark = pytest.mark.skipif(
    psycopg is None or not os.getenv("TEST_DATABASE_URL"),
    reason="psycopg or TEST_DATABASE_URL not available",
)


@pytest.fixture
def connection_scope():
    database_url = os.environ["TEST_DATABASE_URL"]

    @contextmanager
    def _scope():
        assert psycopg is not None
        with psycopg.connect(database_url) as conn:
            yield conn

    return _scope


@pytest.fixture
def store(connection_scope):
    schema_sql = (ROOT / "packages/interacciones/graph/schema_trace.sql").read_text(encoding="utf-8")
    with connection_scope() as cx:
        with cx.cursor() as cur:
            cur.execute("DROP TABLE IF EXISTS workflow_trace_events")
            cur.execute(schema_sql)
            cur.execute("DELETE FROM workflow_trace_events WHERE workflow_id LIKE 'test-wf-%'")
        cx.commit()

    yield PostgresWorkflowTraceStore(connection_scope)

    with connection_scope() as cx:
        with cx.cursor() as cur:
            cur.execute("DELETE FROM workflow_trace_events WHERE workflow_id LIKE 'test-wf-%'")
        cx.commit()


def test_postgres_trace_store_appends_and_reads_hot_trace_events(store: PostgresWorkflowTraceStore) -> None:
    store.append(
        {
            "trace_id": "trace-1",
            "workflow_id": "test-wf-1",
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
            "workflow_id": "test-wf-1",
            "run_id": "run-1",
            "step_id": "dispatch-parse",
            "event_type": "execution.progress",
            "occurred_at": "2026-03-08T12:00:02Z",
            "request_id": "req-1",
            "executor_id": "executor://python-primary",
            "payload": {"progress": 0.5},
        }
    )

    assert store.list_events(workflow_id="test-wf-1", run_id="run-1") == [
        {
            "trace_id": "trace-1",
            "workflow_id": "test-wf-1",
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
            "workflow_id": "test-wf-1",
            "run_id": "run-1",
            "step_id": "dispatch-parse",
            "event_type": "execution.progress",
            "occurred_at": "2026-03-08T12:00:02Z",
            "request_id": "req-1",
            "executor_id": "executor://python-primary",
            "payload": {"progress": 0.5},
        },
    ]


def test_postgres_trace_store_filters_by_workflow_and_run(store: PostgresWorkflowTraceStore) -> None:
    store.append(
        {
            "trace_id": "trace-1",
            "workflow_id": "test-wf-1",
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
            "workflow_id": "test-wf-1",
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
            "workflow_id": "test-wf-2",
            "run_id": "run-1",
            "step_id": "dispatch-parse",
            "event_type": "execution.dispatched",
            "occurred_at": "2026-03-08T12:00:02Z",
            "payload": {},
        }
    )

    assert [event["trace_id"] for event in store.list_events(workflow_id="test-wf-1")] == ["trace-1", "trace-2"]
    assert [event["trace_id"] for event in store.list_events(workflow_id="test-wf-1", run_id="run-2")] == ["trace-2"]
