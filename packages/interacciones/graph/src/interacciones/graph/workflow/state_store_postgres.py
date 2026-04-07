from __future__ import annotations

import json
from typing import Any


class PostgresWorkflowStateStore:
    def __init__(self, connection_scope_fn: Any) -> None:
        self._connection_scope = connection_scope_fn

    def record_dispatch(
        self,
        *,
        workflow_id: str,
        step_id: str,
        executor_id: str,
        request_id: str,
        capability: str,
        policy: dict[str, Any],
        constraints: dict[str, Any],
        payload: dict[str, Any],
    ) -> None:
        self._upsert(
            workflow_id=workflow_id,
            step_id=step_id,
            status="dispatched",
            executor_id=executor_id,
            payload={
                "request_id": request_id,
                "capability": capability,
                "policy": policy,
                "constraints": constraints,
                "payload": payload,
            },
        )

    def record_completion(
        self,
        *,
        workflow_id: str,
        step_id: str,
        executor_id: str,
        result: dict[str, Any],
        artifacts: list[str],
        stdout_lines: list[str] | None = None,
        stderr_lines: list[str] | None = None,
    ) -> None:
        stdout_lines = list(stdout_lines or [])
        stderr_lines = list(stderr_lines or [])
        self._upsert(
            workflow_id=workflow_id,
            step_id=step_id,
            status="completed",
            executor_id=executor_id,
            payload={
                "result": result,
                "artifacts": artifacts,
                "stdout_lines": stdout_lines,
                "stderr_lines": stderr_lines,
            },
        )

    def record_progress(
        self,
        *,
        workflow_id: str,
        step_id: str,
        executor_id: str,
        status: str,
        progress: float,
        message: str | None,
        stdout_lines: list[str] | None = None,
        stderr_lines: list[str] | None = None,
    ) -> None:
        stdout_lines = list(stdout_lines or [])
        stderr_lines = list(stderr_lines or [])
        self._upsert(
            workflow_id=workflow_id,
            step_id=step_id,
            status=status,
            executor_id=executor_id,
            payload={
                "progress": progress,
                "message": message,
                "stdout_lines": stdout_lines,
                "stderr_lines": stderr_lines,
            },
            allow_if_terminal=False,
        )

    def record_failure(
        self,
        *,
        workflow_id: str,
        step_id: str,
        executor_id: str,
        error_code: str,
        error_message: str,
        retryable: bool,
        stdout_lines: list[str] | None = None,
        stderr_lines: list[str] | None = None,
    ) -> None:
        stdout_lines = list(stdout_lines or [])
        stderr_lines = list(stderr_lines or [])
        self._upsert(
            workflow_id=workflow_id,
            step_id=step_id,
            status="failed",
            executor_id=executor_id,
            payload={
                "error_code": error_code,
                "error_message": error_message,
                "retryable": retryable,
                "stdout_lines": stdout_lines,
                "stderr_lines": stderr_lines,
            },
            allow_if_terminal=False,
        )

    def record_approval_requested(
        self,
        *,
        workflow_id: str,
        step_id: str,
        approval_id: str,
        requested_by: str,
        summary: str,
        details: dict[str, Any],
    ) -> None:
        self._upsert(
            workflow_id=workflow_id,
            step_id=step_id,
            status="pending_approval",
            executor_id="",
            payload={
                "approval_id": approval_id,
                "requested_by": requested_by,
                "summary": summary,
                "details": details,
            },
            allow_if_terminal=False,
        )

    def record_approval_resolved(
        self,
        *,
        workflow_id: str,
        step_id: str,
        approval_id: str,
        resolved_by: str,
        approved: bool,
        comment: str | None,
    ) -> None:
        self._upsert(
            workflow_id=workflow_id,
            step_id=step_id,
            status="approved" if approved else "rejected",
            executor_id="",
            payload={
                "approval_id": approval_id,
                "resolved_by": resolved_by,
                "approved": approved,
                "comment": comment,
            },
            allow_if_terminal=False,
        )

    def record_retry_metadata(
        self,
        *,
        workflow_id: str,
        step_id: str,
        retry_count: int,
        max_retries: int,
        deadline_at: str | None,
    ) -> None:
        self._upsert(
            workflow_id=workflow_id,
            step_id=step_id,
            status="retry_scheduled",
            executor_id="",
            payload={
                "retry_count": retry_count,
                "max_retries": max_retries,
                "deadline_at": deadline_at,
            },
            next_run_at=deadline_at,
            allow_if_terminal=False,
        )

    def claim_due_work(
        self,
        *,
        now: str,
        lease_owner: str,
        lease_expires_at: str,
    ) -> list[dict[str, Any]]:
        with self._connection_scope() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    """
                    UPDATE workflow_state
                    SET lease_owner = %s,
                        lease_expires_at = %s::timestamptz,
                        updated_at = NOW()
                    WHERE workflow_id IN (
                        SELECT workflow_id
                        FROM workflow_state
                        WHERE status = 'retry_scheduled'
                          AND next_run_at IS NOT NULL
                          AND next_run_at <= %s::timestamptz
                          AND (lease_expires_at IS NULL OR lease_expires_at <= %s::timestamptz)
                    )
                    RETURNING workflow_id, current_step, status, executor_id, payload, lease_owner, lease_expires_at
                    """,
                    (lease_owner, lease_expires_at, now, now),
                )
                rows = cur.fetchall()
            cx.commit()
        claimed: list[dict[str, Any]] = []
        for row in rows:
            payload = row[4] if isinstance(row, tuple) else row["payload"]
            if isinstance(payload, str):
                payload = json.loads(payload)
            record = {
                "workflow_id": row[0] if isinstance(row, tuple) else row["workflow_id"],
                "current_step": row[1] if isinstance(row, tuple) else row["current_step"],
                "status": row[2] if isinstance(row, tuple) else row["status"],
                **payload,
                "lease_owner": row[5] if isinstance(row, tuple) else row["lease_owner"],
                "lease_expires_at": _normalize_timestamp(row[6] if isinstance(row, tuple) else row["lease_expires_at"]),
            }
            executor_id = row[3] if isinstance(row, tuple) else row["executor_id"]
            if executor_id:
                record["executor_id"] = executor_id
            claimed.append(record)
        return claimed

    def release_lease(self, *, workflow_id: str, lease_owner: str) -> None:
        with self._connection_scope() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    """
                    UPDATE workflow_state
                    SET lease_owner = NULL,
                        lease_expires_at = NULL,
                        updated_at = NOW()
                    WHERE workflow_id = %s
                      AND lease_owner = %s
                    """,
                    (workflow_id, lease_owner),
                )
            cx.commit()

    def renew_lease(self, *, workflow_id: str, lease_owner: str, lease_expires_at: str) -> None:
        with self._connection_scope() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    """
                    UPDATE workflow_state
                    SET lease_expires_at = %s::timestamptz,
                        updated_at = NOW()
                    WHERE workflow_id = %s
                      AND lease_owner = %s
                    """,
                    (lease_expires_at, workflow_id, lease_owner),
                )
            cx.commit()

    def get(self, workflow_id: str) -> dict[str, Any] | None:
        with self._connection_scope() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    "SELECT workflow_id, current_step, status, executor_id, payload, lease_owner, lease_expires_at FROM workflow_state WHERE workflow_id = %s",
                    (workflow_id,),
                )
                row = cur.fetchone()
        if not row:
            return None
        payload = row[4] if isinstance(row, tuple) else row["payload"]
        if isinstance(payload, str):
            payload = json.loads(payload)
        executor_id = row[3] if isinstance(row, tuple) else row["executor_id"]
        lease_owner = row[5] if isinstance(row, tuple) else row["lease_owner"]
        lease_expires_at = row[6] if isinstance(row, tuple) else row["lease_expires_at"]
        state = {
            "workflow_id": row[0] if isinstance(row, tuple) else row["workflow_id"],
            "current_step": row[1] if isinstance(row, tuple) else row["current_step"],
            "status": row[2] if isinstance(row, tuple) else row["status"],
            **payload,
        }
        if executor_id:
            state["executor_id"] = executor_id
        if lease_owner:
            state["lease_owner"] = lease_owner
        if lease_expires_at:
            state["lease_expires_at"] = _normalize_timestamp(lease_expires_at)
        return state

    def _upsert(
        self,
        *,
        workflow_id: str,
        step_id: str,
        status: str,
        executor_id: str,
        payload: dict[str, Any],
        next_run_at: str | None = None,
        allow_if_terminal: bool = True,
    ) -> None:
        with self._connection_scope() as cx:
            with cx.cursor() as cur:
                current_payload: dict[str, Any] = {}
                if not allow_if_terminal:
                    cur.execute("SELECT status FROM workflow_state WHERE workflow_id = %s", (workflow_id,))
                    current = cur.fetchone()
                    if current:
                        current_status = current[0] if isinstance(current, tuple) else current["status"]
                        if current_status in {"completed", "failed"}:
                            cx.commit()
                            return
                cur.execute("SELECT payload FROM workflow_state WHERE workflow_id = %s", (workflow_id,))
                current_payload_row = cur.fetchone()
                if current_payload_row:
                    current_payload = current_payload_row[0] if isinstance(current_payload_row, tuple) else current_payload_row["payload"]
                    if isinstance(current_payload, str):
                        current_payload = json.loads(current_payload)
                payload = {
                    **_request_context(current_payload),
                    **payload,
                    "stdout_lines": [*(current_payload.get("stdout_lines") or []), *(payload.get("stdout_lines") or [])],
                    "stderr_lines": [*(current_payload.get("stderr_lines") or []), *(payload.get("stderr_lines") or [])],
                }
                cur.execute(
                    """
                    INSERT INTO workflow_state (workflow_id, current_step, status, executor_id, payload, next_run_at, created_at, updated_at)
                    VALUES (%s, %s, %s, %s, %s::jsonb, %s::timestamptz, NOW(), NOW())
                    ON CONFLICT (workflow_id)
                    DO UPDATE SET
                        current_step = EXCLUDED.current_step,
                        status = EXCLUDED.status,
                        executor_id = EXCLUDED.executor_id,
                        payload = EXCLUDED.payload,
                        next_run_at = EXCLUDED.next_run_at,
                        lease_owner = NULL,
                        lease_expires_at = NULL,
                        updated_at = NOW()
                    """,
                    (workflow_id, step_id, status, executor_id, json.dumps(payload), next_run_at),
                )
            cx.commit()


def _normalize_timestamp(value: Any) -> str:
    if hasattr(value, "isoformat"):
        return value.isoformat().replace("+00:00", "Z")
    return str(value)


def _request_context(payload: dict[str, Any]) -> dict[str, Any]:
    return {
        key: payload[key]
        for key in ("request_id", "capability", "policy", "constraints", "payload")
        if key in payload
    }
