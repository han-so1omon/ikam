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

from interacciones.graph.workflow.trace_promotion_sink import InMemoryTracePromotionSink
from interacciones.graph.workflow.trace_promotion_sink_postgres import PostgresTracePromotionOutboxSink
from interacciones.graph.workflow.trace_promotion_worker import TracePromotionWorker


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
def outbox_sink(connection_scope):
    schema_sql = (ROOT / "packages/interacciones/graph/schema_trace_promotion_outbox.sql").read_text(encoding="utf-8")
    with connection_scope() as cx:
        with cx.cursor() as cur:
            cur.execute("DROP TABLE IF EXISTS workflow_trace_promotion_outbox")
            cur.execute(schema_sql)
            cur.execute("DELETE FROM workflow_trace_promotion_outbox WHERE workflow_id LIKE 'test-wf-%'")
        cx.commit()

    yield PostgresTracePromotionOutboxSink(connection_scope)

    with connection_scope() as cx:
        with cx.cursor() as cur:
            cur.execute("DELETE FROM workflow_trace_promotion_outbox WHERE workflow_id LIKE 'test-wf-%'")
        cx.commit()


def test_trace_promotion_worker_processes_one_postgres_outbox_item(
    outbox_sink: PostgresTracePromotionOutboxSink,
) -> None:
    outbox_sink.record(
        {
            "workflow_id": "test-wf-worker-1",
            "run_id": "run-1",
            "policy_mode": "on_failure",
            "trigger_event_type": "execution.failed",
            "trace_ids": ["trace-1", "trace-2"],
            "event_count": 2,
        }
    )
    promotion_sink = InMemoryTracePromotionSink()
    worker = TracePromotionWorker(outbox_sink=outbox_sink, promotion_sink=promotion_sink)

    processed = worker.run_once(limit=1, lease_owner="worker-a", lease_expires_at="2099-03-08T12:05:00Z")

    assert len(processed) == 1
    assert processed[0]["workflow_id"] == "test-wf-worker-1"
    assert promotion_sink.plans == processed
    assert outbox_sink.claim_pending(limit=10, lease_owner="worker-b", lease_expires_at="2099-03-08T12:10:00Z") == []
