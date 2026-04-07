from __future__ import annotations

import json
from typing import Any


class PostgresWorkflowTraceStore:
    def __init__(self, connection_scope_fn: Any) -> None:
        self._connection_scope = connection_scope_fn

    def append(self, event: dict[str, Any]) -> None:
        with self._connection_scope() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    """
                    INSERT INTO workflow_trace_events (
                        trace_id,
                        workflow_id,
                        run_id,
                        step_id,
                        event_type,
                        occurred_at,
                        transition_id,
                        marking_before_ref,
                        marking_after_ref,
                        enabled_transition_ids,
                        request_id,
                        executor_id,
                        approval_id,
                        lease_owner,
                        payload,
                        created_at
                    )
                    VALUES (
                        %s, %s, %s, %s, %s, %s::timestamptz,
                        %s, %s, %s, %s::jsonb,
                        %s, %s, %s, %s, %s::jsonb, NOW()
                    )
                    """,
                    (
                        event["trace_id"],
                        event["workflow_id"],
                        event.get("run_id"),
                        event.get("step_id"),
                        event["event_type"],
                        event["occurred_at"],
                        event.get("transition_id"),
                        event.get("marking_before_ref"),
                        event.get("marking_after_ref"),
                        json.dumps(event.get("enabled_transition_ids")) if "enabled_transition_ids" in event else None,
                        event.get("request_id"),
                        event.get("executor_id"),
                        event.get("approval_id"),
                        event.get("lease_owner"),
                        json.dumps(event.get("payload", {})),
                    ),
                )
            cx.commit()

    def list_events(self, *, workflow_id: str, run_id: str | None = None) -> list[dict[str, Any]]:
        with self._connection_scope() as cx:
            with cx.cursor() as cur:
                if run_id is None:
                    cur.execute(
                        """
                        SELECT trace_id, workflow_id, run_id, step_id, event_type, occurred_at,
                               transition_id, marking_before_ref, marking_after_ref, enabled_transition_ids,
                               request_id, executor_id, approval_id, lease_owner, payload
                        FROM workflow_trace_events
                        WHERE workflow_id = %s
                        ORDER BY occurred_at ASC, trace_id ASC
                        """,
                        (workflow_id,),
                    )
                else:
                    cur.execute(
                        """
                        SELECT trace_id, workflow_id, run_id, step_id, event_type, occurred_at,
                               transition_id, marking_before_ref, marking_after_ref, enabled_transition_ids,
                               request_id, executor_id, approval_id, lease_owner, payload
                        FROM workflow_trace_events
                        WHERE workflow_id = %s AND run_id = %s
                        ORDER BY occurred_at ASC, trace_id ASC
                        """,
                        (workflow_id, run_id),
                    )
                rows = cur.fetchall()
        return [_to_event(row) for row in rows]


def _to_event(row: Any) -> dict[str, Any]:
    payload = row[14] if isinstance(row, tuple) else row["payload"]
    if isinstance(payload, str):
        payload = json.loads(payload)
    enabled_transition_ids = row[9] if isinstance(row, tuple) else row["enabled_transition_ids"]
    if isinstance(enabled_transition_ids, str):
        enabled_transition_ids = json.loads(enabled_transition_ids)
    event = {
        "trace_id": row[0] if isinstance(row, tuple) else row["trace_id"],
        "workflow_id": row[1] if isinstance(row, tuple) else row["workflow_id"],
        "event_type": row[4] if isinstance(row, tuple) else row["event_type"],
        "occurred_at": _normalize_timestamp(row[5] if isinstance(row, tuple) else row["occurred_at"]),
        "payload": payload,
    }
    for key, index in (
        ("run_id", 2),
        ("step_id", 3),
        ("transition_id", 6),
        ("marking_before_ref", 7),
        ("marking_after_ref", 8),
        ("request_id", 10),
        ("executor_id", 11),
        ("approval_id", 12),
        ("lease_owner", 13),
    ):
        value = row[index] if isinstance(row, tuple) else row[key]
        if value is not None:
            event[key] = value
    if enabled_transition_ids is not None:
        event["enabled_transition_ids"] = enabled_transition_ids
    return event


def _normalize_timestamp(value: Any) -> str:
    if hasattr(value, "isoformat"):
        return value.isoformat().replace("+00:00", "Z")
    return str(value)
