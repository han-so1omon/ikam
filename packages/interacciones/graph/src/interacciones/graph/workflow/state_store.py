from __future__ import annotations

from datetime import datetime, timezone
from typing import Any


class InMemoryWorkflowStateStore:
    def __init__(self) -> None:
        self._state: dict[str, dict[str, Any]] = {}

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
        self._state[workflow_id] = {
            "workflow_id": workflow_id,
            "current_step": step_id,
            "status": "dispatched",
            "executor_id": executor_id,
            "request_id": request_id,
            "capability": capability,
            "policy": policy,
            "constraints": constraints,
            "payload": payload,
        }

    def record_queued(
        self,
        *,
        workflow_id: str,
        step_id: str,
        executor_id: str,
        executor_kind: str,
        request_id: str,
        capability: str,
    ) -> None:
        if self._is_terminal(workflow_id):
            return
        self._state[workflow_id] = {
            **self._request_context(workflow_id),
            "workflow_id": workflow_id,
            "current_step": step_id,
            "status": "queued",
            "executor_id": executor_id,
            "executor_kind": executor_kind,
            "request_id": request_id,
            "capability": capability,
        }

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
        if self._is_terminal(workflow_id):
            return
        current = self._state.get(workflow_id, {})
        stdout_lines = list(stdout_lines or [])
        stderr_lines = list(stderr_lines or [])
        self._state[workflow_id] = {
            **self._request_context(workflow_id),
            "workflow_id": workflow_id,
            "current_step": step_id,
            "status": status,
            "executor_id": executor_id,
            "progress": progress,
            "message": message,
            "stdout_lines": [*current.get("stdout_lines", []), *stdout_lines],
            "stderr_lines": [*current.get("stderr_lines", []), *stderr_lines],
        }

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
        current = self._state.get(workflow_id, {})
        stdout_lines = list(stdout_lines or [])
        stderr_lines = list(stderr_lines or [])
        self._state[workflow_id] = {
            **self._request_context(workflow_id),
            "workflow_id": workflow_id,
            "current_step": step_id,
            "status": "completed",
            "executor_id": executor_id,
            "result": result,
            "artifacts": artifacts,
            "stdout_lines": [*current.get("stdout_lines", []), *stdout_lines],
            "stderr_lines": [*current.get("stderr_lines", []), *stderr_lines],
        }

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
        if self._is_terminal(workflow_id):
            return
        current = self._state.get(workflow_id, {})
        stdout_lines = list(stdout_lines or [])
        stderr_lines = list(stderr_lines or [])
        self._state[workflow_id] = {
            **self._request_context(workflow_id),
            "workflow_id": workflow_id,
            "current_step": step_id,
            "status": "failed",
            "executor_id": executor_id,
            "error_code": error_code,
            "error_message": error_message,
            "retryable": retryable,
            "stdout_lines": [*current.get("stdout_lines", []), *stdout_lines],
            "stderr_lines": [*current.get("stderr_lines", []), *stderr_lines],
        }

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
        if self._is_terminal(workflow_id):
            return
        self._state[workflow_id] = {
            **self._request_context(workflow_id),
            "workflow_id": workflow_id,
            "current_step": step_id,
            "status": "pending_approval",
            "approval_id": approval_id,
            "requested_by": requested_by,
            "summary": summary,
            "details": details,
        }

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
        if self._is_terminal(workflow_id):
            return
        self._state[workflow_id] = {
            **self._request_context(workflow_id),
            "workflow_id": workflow_id,
            "current_step": step_id,
            "status": "approved" if approved else "rejected",
            "approval_id": approval_id,
            "resolved_by": resolved_by,
            "approved": approved,
            "comment": comment,
        }

    def record_retry_metadata(
        self,
        *,
        workflow_id: str,
        step_id: str,
        retry_count: int,
        max_retries: int,
        deadline_at: str | None,
    ) -> None:
        if self._is_terminal(workflow_id):
            return
        self._state[workflow_id] = {
            **self._request_context(workflow_id),
            "workflow_id": workflow_id,
            "current_step": step_id,
            "status": "retry_scheduled",
            "retry_count": retry_count,
            "max_retries": max_retries,
            "deadline_at": deadline_at,
        }

    def claim_due_work(
        self,
        *,
        now: str,
        lease_owner: str,
        lease_expires_at: str,
    ) -> list[dict[str, Any]]:
        now_dt = _parse_iso(now)
        claimed: list[dict[str, Any]] = []
        for workflow_id, state in self._state.items():
            deadline_at = state.get("deadline_at")
            if state.get("status") != "retry_scheduled" or not deadline_at:
                continue
            deadline_dt = _parse_iso(deadline_at)
            lease_expiry_raw = state.get("lease_expires_at")
            if deadline_dt > now_dt:
                continue
            if lease_expiry_raw and _parse_iso(lease_expiry_raw) > now_dt:
                continue
            state["lease_owner"] = lease_owner
            state["lease_expires_at"] = lease_expires_at
            claimed.append(dict(state))
        return claimed

    def release_lease(self, *, workflow_id: str, lease_owner: str) -> None:
        state = self._state.get(workflow_id)
        if not state or state.get("lease_owner") != lease_owner:
            return
        state.pop("lease_owner", None)
        state.pop("lease_expires_at", None)

    def renew_lease(self, *, workflow_id: str, lease_owner: str, lease_expires_at: str) -> None:
        state = self._state.get(workflow_id)
        if not state or state.get("lease_owner") != lease_owner:
            return
        state["lease_expires_at"] = lease_expires_at

    def get(self, workflow_id: str) -> dict[str, Any] | None:
        return self._state.get(workflow_id)

    def _is_terminal(self, workflow_id: str) -> bool:
        current = self._state.get(workflow_id)
        return bool(current and current.get("status") in {"completed", "failed"})

    def _request_context(self, workflow_id: str) -> dict[str, Any]:
        current = self._state.get(workflow_id, {})
        return {
            key: current[key]
            for key in ("request_id", "capability", "executor_kind", "policy", "constraints", "payload")
            if key in current
        }


def _parse_iso(value: str) -> datetime:
    return datetime.fromisoformat(value.replace("Z", "+00:00")).astimezone(timezone.utc)
