from __future__ import annotations

from typing import Any, Protocol


class WorkflowStateStore(Protocol):
    def record_queued(
        self,
        *,
        workflow_id: str,
        step_id: str,
        executor_id: str,
        executor_kind: str,
        request_id: str,
        capability: str,
    ) -> None: ...

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
    ) -> None: ...

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
    ) -> None: ...

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
    ) -> None: ...

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
    ) -> None: ...

    def record_approval_requested(
        self,
        *,
        workflow_id: str,
        step_id: str,
        approval_id: str,
        requested_by: str,
        summary: str,
        details: dict[str, Any],
    ) -> None: ...

    def record_approval_resolved(
        self,
        *,
        workflow_id: str,
        step_id: str,
        approval_id: str,
        resolved_by: str,
        approved: bool,
        comment: str | None,
    ) -> None: ...

    def record_retry_metadata(
        self,
        *,
        workflow_id: str,
        step_id: str,
        retry_count: int,
        max_retries: int,
        deadline_at: str | None,
    ) -> None: ...

    def claim_due_work(
        self,
        *,
        now: str,
        lease_owner: str,
        lease_expires_at: str,
    ) -> list[dict[str, Any]]: ...

    def release_lease(self, *, workflow_id: str, lease_owner: str) -> None: ...

    def renew_lease(self, *, workflow_id: str, lease_owner: str, lease_expires_at: str) -> None: ...

    def get(self, workflow_id: str) -> dict[str, Any] | None: ...
