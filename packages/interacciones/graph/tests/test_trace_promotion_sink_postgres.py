from __future__ import annotations

import os
from contextlib import contextmanager
from pathlib import Path
import sys
import threading

import pytest

try:
    import psycopg
except ImportError:  # pragma: no cover - environment dependent
    psycopg = None


ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "packages/interacciones/graph/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))

from interacciones.graph.workflow.trace_promotion_sink_postgres import PostgresTracePromotionOutboxSink


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
def sink(connection_scope):
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


def test_postgres_trace_promotion_outbox_sink_records_plan(sink: PostgresTracePromotionOutboxSink) -> None:
    sink.record(
        {
            "workflow_id": "test-wf-1",
            "run_id": "run-1",
            "target_ref": "refs/heads/run/run-1",
            "policy_mode": "on_failure",
            "trigger_event_type": "execution.failed",
            "trace_ids": ["trace-1", "trace-2"],
            "event_count": 2,
        }
    )

    assert sink.list_records(workflow_id="test-wf-1") == [
        {
            "workflow_id": "test-wf-1",
            "run_id": "run-1",
            "target_ref": "refs/heads/run/run-1",
            "policy_mode": "on_failure",
            "trigger_event_type": "execution.failed",
            "trace_ids": ["trace-1", "trace-2"],
            "event_count": 2,
        }
    ]


def test_postgres_trace_promotion_outbox_sink_records_committed_fragment_id_when_writer_returns_one(
    connection_scope,
) -> None:
    schema_sql = (ROOT / "packages/interacciones/graph/schema_trace_promotion_outbox.sql").read_text(encoding="utf-8")
    with connection_scope() as cx:
        with cx.cursor() as cur:
            cur.execute("DROP TABLE IF EXISTS workflow_trace_promotion_outbox")
            cur.execute(schema_sql)
        cx.commit()

    sink = PostgresTracePromotionOutboxSink(connection_scope, writer=lambda plan: "fragment://trace-bundle-db-1")
    sink.record(
        {
            "workflow_id": "test-wf-2",
            "run_id": "run-2",
            "target_ref": "refs/heads/main",
            "policy_mode": "on_failure",
            "trigger_event_type": "execution.failed",
            "trace_ids": ["trace-3", "trace-4"],
            "event_count": 2,
        }
    )

    assert sink.list_records(workflow_id="test-wf-2") == [
        {
            "workflow_id": "test-wf-2",
            "run_id": "run-2",
            "target_ref": "refs/heads/main",
            "policy_mode": "on_failure",
            "trigger_event_type": "execution.failed",
            "trace_ids": ["trace-3", "trace-4"],
            "event_count": 2,
            "committed_trace_fragment_id": "fragment://trace-bundle-db-1",
        }
    ]


def test_postgres_trace_promotion_outbox_sink_claims_oldest_pending_records_and_acks_them(
    sink: PostgresTracePromotionOutboxSink,
) -> None:
    sink.record(
        {
            "workflow_id": "test-wf-3",
            "run_id": "run-1",
            "policy_mode": "on_failure",
            "trigger_event_type": "execution.failed",
            "trace_ids": ["trace-1"],
            "event_count": 1,
        }
    )
    sink.record(
        {
            "workflow_id": "test-wf-4",
            "run_id": "run-1",
            "policy_mode": "on_failure",
            "trigger_event_type": "execution.failed",
            "trace_ids": ["trace-2"],
            "event_count": 1,
        }
    )

    claimed = sink.claim_pending(limit=1, lease_owner="worker-a", lease_expires_at="2099-03-08T12:05:00Z")

    assert len(claimed) == 1
    assert claimed[0]["workflow_id"] == "test-wf-3"
    assert claimed[0]["outbox_id"]
    assert claimed[0]["lease_owner"] == "worker-a"

    sink.ack(claimed[0]["outbox_id"], lease_owner="worker-a")

    remaining = sink.claim_pending(limit=10, lease_owner="worker-b", lease_expires_at="2099-03-08T12:10:00Z")
    assert [item["workflow_id"] for item in remaining] == ["test-wf-4"]


def test_postgres_trace_promotion_outbox_sink_rejects_ack_from_different_lease_owner(
    sink: PostgresTracePromotionOutboxSink,
) -> None:
    sink.record(
        {
            "workflow_id": "test-wf-5",
            "run_id": "run-1",
            "policy_mode": "on_failure",
            "trigger_event_type": "execution.failed",
            "trace_ids": ["trace-9"],
            "event_count": 1,
        }
    )

    [claimed] = sink.claim_pending(limit=1, lease_owner="worker-a", lease_expires_at="2099-03-08T12:05:00Z")

    sink.ack(claimed["outbox_id"], lease_owner="worker-b")

    still_claimed = sink.claim_pending(limit=10, lease_owner="worker-c", lease_expires_at="2099-03-08T12:10:00Z")
    assert still_claimed == []


def test_postgres_trace_promotion_outbox_sink_releases_and_renews_matching_leases(
    sink: PostgresTracePromotionOutboxSink,
) -> None:
    sink.record(
        {
            "workflow_id": "test-wf-6",
            "run_id": "run-1",
            "policy_mode": "on_failure",
            "trigger_event_type": "execution.failed",
            "trace_ids": ["trace-10"],
            "event_count": 1,
        }
    )

    [claimed] = sink.claim_pending(limit=1, lease_owner="worker-a", lease_expires_at="2099-03-08T12:05:00Z")

    sink.renew_lease(claimed["outbox_id"], lease_owner="worker-a", lease_expires_at="2099-03-08T12:10:00Z")
    sink.release_lease(claimed["outbox_id"], lease_owner="worker-a")

    [reclaimed] = sink.claim_pending(limit=1, lease_owner="worker-b", lease_expires_at="2026-03-08T12:15:00Z")
    assert reclaimed["workflow_id"] == "test-wf-6"
    assert reclaimed["lease_owner"] == "worker-b"


def test_postgres_trace_promotion_outbox_sink_reclaims_expired_leases(
    sink: PostgresTracePromotionOutboxSink,
) -> None:
    sink.record(
        {
            "workflow_id": "test-wf-7",
            "run_id": "run-1",
            "policy_mode": "on_failure",
            "trigger_event_type": "execution.failed",
            "trace_ids": ["trace-11"],
            "event_count": 1,
        }
    )

    [claimed] = sink.claim_pending(limit=1, lease_owner="worker-a", lease_expires_at="2000-01-01T00:00:00Z")

    [reclaimed] = sink.claim_pending(limit=1, lease_owner="worker-b", lease_expires_at="2026-03-08T12:15:00Z")
    assert claimed["outbox_id"] == reclaimed["outbox_id"]
    assert reclaimed["lease_owner"] == "worker-b"


def test_postgres_trace_promotion_outbox_sink_does_not_double_claim_active_lease(
    sink: PostgresTracePromotionOutboxSink,
) -> None:
    sink.record(
        {
            "workflow_id": "test-wf-8",
            "run_id": "run-1",
            "policy_mode": "final_only",
            "trigger_event_type": "execution.completed",
            "trace_ids": ["trace-12"],
            "event_count": 1,
        }
    )

    claimed_by_worker_a = []
    claimed_by_worker_b = []
    ready = threading.Barrier(2)

    def _claim_into(target: list[dict[str, object]], owner: str) -> None:
        ready.wait()
        target.extend(
            sink.claim_pending(
                limit=1,
                lease_owner=owner,
                lease_expires_at="2099-03-08T12:05:00Z",
            )
        )

    thread_a = threading.Thread(target=_claim_into, args=(claimed_by_worker_a, "worker-a"))
    thread_b = threading.Thread(target=_claim_into, args=(claimed_by_worker_b, "worker-b"))

    thread_a.start()
    thread_b.start()
    thread_a.join()
    thread_b.join()

    all_claims = claimed_by_worker_a + claimed_by_worker_b

    assert len(all_claims) == 1
    assert all_claims[0]["workflow_id"] == "test-wf-8"
    assert {claim["lease_owner"] for claim in all_claims} <= {"worker-a", "worker-b"}
