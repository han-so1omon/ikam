from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "packages/interacciones/graph/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))

from interacciones.graph.workflow.scheduler import WorkflowScheduler
from interacciones.graph.workflow.state_store import InMemoryWorkflowStateStore


def test_workflow_scheduler_emits_retry_wakeup_actions_for_due_work() -> None:
    store = InMemoryWorkflowStateStore()
    store.record_retry_metadata(
        workflow_id="wf-1",
        step_id="dispatch-parse",
        retry_count=1,
        max_retries=3,
        deadline_at="2026-03-08T12:00:00Z",
    )
    scheduler = WorkflowScheduler(store, lease_owner="scheduler-a")

    claimed = scheduler.tick_due_work(now="2026-03-08T12:00:00Z", lease_duration_seconds=300)

    assert claimed == [
        {
            "kind": "retry_wakeup",
            "workflow_id": "wf-1",
            "step_id": "dispatch-parse",
            "retry_count": 1,
            "max_retries": 3,
            "deadline_at": "2026-03-08T12:00:00Z",
            "lease_owner": "scheduler-a",
            "lease_expires_at": "2026-03-08T12:05:00Z",
        }
    ]


def test_workflow_scheduler_returns_empty_when_no_due_work() -> None:
    store = InMemoryWorkflowStateStore()
    store.record_retry_metadata(
        workflow_id="wf-1",
        step_id="dispatch-parse",
        retry_count=1,
        max_retries=3,
        deadline_at="2026-03-08T12:00:00Z",
    )
    scheduler = WorkflowScheduler(store, lease_owner="scheduler-a")

    claimed = scheduler.tick_due_work(now="2026-03-08T11:59:00Z", lease_duration_seconds=300)

    assert claimed == []


def test_in_memory_workflow_state_store_supports_lease_release_and_reclaim() -> None:
    store = InMemoryWorkflowStateStore()
    store.record_retry_metadata(
        workflow_id="wf-2",
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

    store.release_lease(workflow_id="wf-2", lease_owner="scheduler-a")

    second_claim = store.claim_due_work(
        now="2026-03-08T12:01:00Z",
        lease_owner="scheduler-b",
        lease_expires_at="2026-03-08T12:06:00Z",
    )
    assert second_claim == [
        {
            "workflow_id": "wf-2",
            "current_step": "dispatch-parse",
            "status": "retry_scheduled",
            "retry_count": 1,
            "max_retries": 3,
            "deadline_at": "2026-03-08T12:00:00Z",
            "lease_owner": "scheduler-b",
            "lease_expires_at": "2026-03-08T12:06:00Z",
        }
    ]


def test_in_memory_workflow_state_store_supports_lease_renewal_and_expiry_recovery() -> None:
    store = InMemoryWorkflowStateStore()
    store.record_retry_metadata(
        workflow_id="wf-3",
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
        workflow_id="wf-3",
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
            "workflow_id": "wf-3",
            "current_step": "dispatch-parse",
            "status": "retry_scheduled",
            "retry_count": 1,
            "max_retries": 3,
            "deadline_at": "2026-03-08T12:00:00Z",
            "lease_owner": "scheduler-b",
            "lease_expires_at": "2026-03-08T12:16:00Z",
        }
    ]
