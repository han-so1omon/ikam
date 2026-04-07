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
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))

from interacciones.graph.workflow.orchestrator import WorkflowOrchestrator
from interacciones.graph.workflow.state_store_postgres import PostgresWorkflowStateStore
from interacciones.schemas.execution import ApprovalRequested, ApprovalResolved, ExecutionRequest
from interacciones.schemas.executors import ExecutorDeclaration


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
    schema_sql = (ROOT / "packages/interacciones/graph/schema_workflow.sql").read_text(encoding="utf-8")
    with connection_scope() as cx:
        with cx.cursor() as cur:
            cur.execute("DROP TABLE IF EXISTS workflow_state")
            cur.execute(schema_sql)
            cur.execute("DELETE FROM workflow_state WHERE workflow_id LIKE 'test-wf-%'")
        cx.commit()

    yield PostgresWorkflowStateStore(connection_scope)

    with connection_scope() as cx:
        with cx.cursor() as cur:
            cur.execute("DELETE FROM workflow_state WHERE workflow_id LIKE 'test-wf-%'")
        cx.commit()


def test_postgres_workflow_state_store_records_dispatch(store: PostgresWorkflowStateStore) -> None:
    store.record_dispatch(
        workflow_id="test-wf-1",
        step_id="dispatch-parse",
        executor_id="executor://python-primary",
        request_id="req-1",
        capability="python.parse_artifacts",
        policy={"cost_tier": "standard"},
        constraints={},
        payload={"input": "hello"},
    )

    assert store.get("test-wf-1") == {
        "workflow_id": "test-wf-1",
        "current_step": "dispatch-parse",
        "status": "dispatched",
        "executor_id": "executor://python-primary",
        "request_id": "req-1",
        "capability": "python.parse_artifacts",
        "policy": {"cost_tier": "standard"},
        "constraints": {},
        "payload": {"input": "hello"},
    }


def test_postgres_workflow_state_store_records_completion(store: PostgresWorkflowStateStore) -> None:
    store.record_completion(
        workflow_id="test-wf-2",
        step_id="dispatch-parse",
        executor_id="executor://python-primary",
        result={"documents": 1},
        artifacts=["artifact://parsed"],
    )

    assert store.get("test-wf-2") == {
        "workflow_id": "test-wf-2",
        "current_step": "dispatch-parse",
        "status": "completed",
        "executor_id": "executor://python-primary",
        "result": {"documents": 1},
        "artifacts": ["artifact://parsed"],
    }


def test_postgres_workflow_state_store_records_progress_and_failure(store: PostgresWorkflowStateStore) -> None:
    store.record_progress(
        workflow_id="test-wf-4",
        step_id="dispatch-parse",
        executor_id="executor://python-primary",
        status="running",
        progress=0.5,
        message="halfway",
    )

    assert store.get("test-wf-4") == {
        "workflow_id": "test-wf-4",
        "current_step": "dispatch-parse",
        "status": "running",
        "executor_id": "executor://python-primary",
        "progress": 0.5,
        "message": "halfway",
    }

    store.record_failure(
        workflow_id="test-wf-4",
        step_id="dispatch-parse",
        executor_id="executor://python-primary",
        error_code="executor_timeout",
        error_message="timed out",
        retryable=True,
    )

    assert store.get("test-wf-4") == {
        "workflow_id": "test-wf-4",
        "current_step": "dispatch-parse",
        "status": "failed",
        "executor_id": "executor://python-primary",
        "error_code": "executor_timeout",
        "error_message": "timed out",
        "retryable": True,
    }


def test_postgres_workflow_state_store_does_not_overwrite_terminal_state_with_progress(store: PostgresWorkflowStateStore) -> None:
    store.record_completion(
        workflow_id="test-wf-5",
        step_id="dispatch-parse",
        executor_id="executor://python-primary",
        result={"documents": 1},
        artifacts=["artifact://parsed"],
    )

    store.record_progress(
        workflow_id="test-wf-5",
        step_id="dispatch-parse",
        executor_id="executor://python-primary",
        status="running",
        progress=0.5,
        message="late-progress",
    )

    assert store.get("test-wf-5") == {
        "workflow_id": "test-wf-5",
        "current_step": "dispatch-parse",
        "status": "completed",
        "executor_id": "executor://python-primary",
        "result": {"documents": 1},
        "artifacts": ["artifact://parsed"],
    }


def test_workflow_orchestrator_can_use_postgres_state_store(connection_scope) -> None:
    schema_sql = (ROOT / "packages/interacciones/graph/schema_workflow.sql").read_text(encoding="utf-8")
    with connection_scope() as cx:
        with cx.cursor() as cur:
            cur.execute(schema_sql)
            cur.execute("DELETE FROM workflow_state WHERE workflow_id = 'test-wf-3'")
        cx.commit()

    store = PostgresWorkflowStateStore(connection_scope)
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

    published = orchestrator.dispatch_execution(
        ExecutionRequest(
            request_id="req-1",
            workflow_id="test-wf-3",
            step_id="dispatch-parse",
            capability="python.parse_artifacts",
            policy={"cost_tier": "standard"},
            constraints={},
            payload={"input": "hello"},
        )
    )

    assert published.topic == "execution.requests"
    assert store.get("test-wf-3") == {
        "workflow_id": "test-wf-3",
        "current_step": "dispatch-parse",
        "status": "dispatched",
        "executor_id": "executor://python-primary",
        "request_id": "req-1",
        "capability": "python.parse_artifacts",
        "policy": {"cost_tier": "standard"},
        "constraints": {},
        "payload": {"input": "hello"},
    }


def test_postgres_workflow_state_store_records_pending_approval_and_resolution(store: PostgresWorkflowStateStore) -> None:
    store.record_approval_requested(
        workflow_id="test-wf-6",
        step_id="await-approval",
        approval_id="approval-1",
        requested_by="dispatcher",
        summary="Need approval",
        details={"reason": "sensitive"},
    )

    assert store.get("test-wf-6") == {
        "workflow_id": "test-wf-6",
        "current_step": "await-approval",
        "status": "pending_approval",
        "approval_id": "approval-1",
        "requested_by": "dispatcher",
        "summary": "Need approval",
        "details": {"reason": "sensitive"},
    }

    store.record_approval_resolved(
        workflow_id="test-wf-6",
        step_id="await-approval",
        approval_id="approval-1",
        resolved_by="reviewer",
        approved=True,
        comment="looks good",
    )

    assert store.get("test-wf-6") == {
        "workflow_id": "test-wf-6",
        "current_step": "await-approval",
        "status": "approved",
        "approval_id": "approval-1",
        "resolved_by": "reviewer",
        "approved": True,
        "comment": "looks good",
    }


def test_postgres_workflow_state_store_records_retry_and_deadline_metadata(store: PostgresWorkflowStateStore) -> None:
    store.record_retry_metadata(
        workflow_id="test-wf-7",
        step_id="dispatch-parse",
        retry_count=2,
        max_retries=5,
        deadline_at="2026-03-08T12:00:00Z",
    )

    assert store.get("test-wf-7") == {
        "workflow_id": "test-wf-7",
        "current_step": "dispatch-parse",
        "status": "retry_scheduled",
        "retry_count": 2,
        "max_retries": 5,
        "deadline_at": "2026-03-08T12:00:00Z",
    }


def test_postgres_workflow_state_store_claims_due_work_with_lease(store: PostgresWorkflowStateStore) -> None:
    store.record_retry_metadata(
        workflow_id="test-wf-8",
        step_id="dispatch-parse",
        retry_count=1,
        max_retries=3,
        deadline_at="2026-03-08T12:00:00Z",
    )

    claimed = store.claim_due_work(
        now="2026-03-08T12:00:00Z",
        lease_owner="scheduler-a",
        lease_expires_at="2026-03-08T12:05:00Z",
    )

    assert claimed == [
        {
            "workflow_id": "test-wf-8",
            "current_step": "dispatch-parse",
            "status": "retry_scheduled",
            "retry_count": 1,
            "max_retries": 3,
            "deadline_at": "2026-03-08T12:00:00Z",
            "lease_owner": "scheduler-a",
            "lease_expires_at": "2026-03-08T12:05:00Z",
        }
    ]

    claimed_again = store.claim_due_work(
        now="2026-03-08T12:01:00Z",
        lease_owner="scheduler-b",
        lease_expires_at="2026-03-08T12:06:00Z",
    )
    assert claimed_again == []


def test_postgres_workflow_state_store_supports_lease_release_and_reclaim(store: PostgresWorkflowStateStore) -> None:
    store.record_retry_metadata(
        workflow_id="test-wf-9",
        step_id="dispatch-parse",
        retry_count=1,
        max_retries=3,
        deadline_at="2026-03-08T12:00:00Z",
    )
    first_claim = store.claim_due_work(
        now="2026-03-08T12:00:00Z",
        lease_owner="scheduler-a",
        lease_expires_at="2026-03-08T12:05:00Z",
    )
    assert len(first_claim) == 1

    store.release_lease(workflow_id="test-wf-9", lease_owner="scheduler-a")

    reclaimed = store.claim_due_work(
        now="2026-03-08T12:01:00Z",
        lease_owner="scheduler-b",
        lease_expires_at="2026-03-08T12:06:00Z",
    )
    assert reclaimed == [
        {
            "workflow_id": "test-wf-9",
            "current_step": "dispatch-parse",
            "status": "retry_scheduled",
            "retry_count": 1,
            "max_retries": 3,
            "deadline_at": "2026-03-08T12:00:00Z",
            "lease_owner": "scheduler-b",
            "lease_expires_at": "2026-03-08T12:06:00Z",
        }
    ]


def test_postgres_workflow_state_store_supports_lease_renewal_and_expiry_recovery(store: PostgresWorkflowStateStore) -> None:
    store.record_retry_metadata(
        workflow_id="test-wf-10",
        step_id="dispatch-parse",
        retry_count=1,
        max_retries=3,
        deadline_at="2026-03-08T12:00:00Z",
    )
    store.claim_due_work(
        now="2026-03-08T12:00:00Z",
        lease_owner="scheduler-a",
        lease_expires_at="2026-03-08T12:05:00Z",
    )
    store.renew_lease(
        workflow_id="test-wf-10",
        lease_owner="scheduler-a",
        lease_expires_at="2026-03-08T12:10:00Z",
    )

    not_yet_reclaimable = store.claim_due_work(
        now="2026-03-08T12:06:00Z",
        lease_owner="scheduler-b",
        lease_expires_at="2026-03-08T12:11:00Z",
    )
    assert not_yet_reclaimable == []

    reclaimable = store.claim_due_work(
        now="2026-03-08T12:11:00Z",
        lease_owner="scheduler-b",
        lease_expires_at="2026-03-08T12:16:00Z",
    )
    assert reclaimable == [
        {
            "workflow_id": "test-wf-10",
            "current_step": "dispatch-parse",
            "status": "retry_scheduled",
            "retry_count": 1,
            "max_retries": 3,
            "deadline_at": "2026-03-08T12:00:00Z",
            "lease_owner": "scheduler-b",
            "lease_expires_at": "2026-03-08T12:16:00Z",
        }
    ]
