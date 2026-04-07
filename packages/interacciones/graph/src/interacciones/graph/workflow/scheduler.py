from __future__ import annotations

from datetime import datetime, timedelta, timezone
from typing import Any

from interacciones.graph.workflow.state_store_protocol import WorkflowStateStore


class WorkflowScheduler:
    def __init__(self, store: WorkflowStateStore, *, lease_owner: str) -> None:
        self._store = store
        self._lease_owner = lease_owner

    def tick_due_work(self, *, now: str, lease_duration_seconds: int) -> list[dict]:
        now_dt = _parse_iso(now)
        lease_expires_at = (now_dt + timedelta(seconds=lease_duration_seconds)).isoformat().replace("+00:00", "Z")
        claimed = self._store.claim_due_work(now=now, lease_owner=self._lease_owner, lease_expires_at=lease_expires_at)
        return [_to_action(item) for item in claimed]


def _parse_iso(value: str) -> datetime:
    return datetime.fromisoformat(value.replace("Z", "+00:00")).astimezone(timezone.utc)


def _to_action(item: dict[str, Any]) -> dict[str, Any]:
    if item.get("status") == "retry_scheduled":
        return {
            "kind": "retry_wakeup",
            "workflow_id": item["workflow_id"],
            "step_id": item["current_step"],
            "retry_count": item["retry_count"],
            "max_retries": item["max_retries"],
            "deadline_at": item.get("deadline_at"),
            "lease_owner": item.get("lease_owner"),
            "lease_expires_at": item.get("lease_expires_at"),
        }
    return dict(item)
