from __future__ import annotations

import io
from contextlib import redirect_stderr, redirect_stdout
from typing import Any, Callable

from interacciones.schemas import ExecutionCompleted, ExecutionFailed, ExecutionProgress, ExecutionQueueRequest


class SharedQueueExecutor:
    def __init__(
        self,
        *,
        executor_id: str,
        executor_kind: str,
        publisher: Any,
        handlers: dict[str, Callable[[dict[str, Any]], dict[str, Any]]],
    ) -> None:
        self._executor_id = executor_id
        self._executor_kind = executor_kind
        self._publisher = publisher
        self._handlers = handlers

    def consume(self, *, topic: str, payload: dict[str, Any]) -> None:
        request = ExecutionQueueRequest.model_validate(payload)
        if topic != request.transport.get("request_topic"):
            raise ValueError(f"unsupported request topic: {topic}")
        if request.executor_id != self._executor_id or request.executor_kind != self._executor_kind:
            return
        handler = self._handlers.get(request.capability)
        if handler is None:
            self._publisher.publish_failed(
                ExecutionFailed(
                    request_id=request.request_id,
                    workflow_id=request.workflow_id,
                    step_id=request.step_id,
                    executor_id=self._executor_id,
                    error_code="unsupported_capability",
                    error_message=f"unsupported capability: {request.capability}",
                    retryable=False,
                )
            )
            return
        self._publisher.publish_progress(
            ExecutionProgress(
                request_id=request.request_id,
                workflow_id=request.workflow_id,
                step_id=request.step_id,
                executor_id=self._executor_id,
                status="running",
                progress=0.0,
                message="started",
                stdout_lines=[],
                stderr_lines=[],
            )
        )
        stdout_buffer = io.StringIO()
        stderr_buffer = io.StringIO()
        try:
            with redirect_stdout(stdout_buffer), redirect_stderr(stderr_buffer):
                result = handler(dict(request.payload))
        except Exception as exc:
            stdout_lines = _captured_lines(stdout_buffer)
            stderr_lines = _captured_lines(stderr_buffer)
            self._publish_log_progress_events(request=request, stdout_lines=stdout_lines, stderr_lines=stderr_lines)
            self._publisher.publish_failed(
                ExecutionFailed(
                    request_id=request.request_id,
                    workflow_id=request.workflow_id,
                    step_id=request.step_id,
                    executor_id=self._executor_id,
                    error_code="execution_error",
                    error_message=str(exc),
                    retryable=False,
                    stdout_lines=[],
                    stderr_lines=[],
                )
            )
            return
        stdout_lines = _captured_lines(stdout_buffer)
        stderr_lines = _captured_lines(stderr_buffer)
        self._publish_log_progress_events(request=request, stdout_lines=stdout_lines, stderr_lines=stderr_lines)
        self._publisher.publish_completed(
            ExecutionCompleted(
                request_id=request.request_id,
                workflow_id=request.workflow_id,
                step_id=request.step_id,
                executor_id=self._executor_id,
                result=result,
                artifacts=[],
                stdout_lines=[],
                stderr_lines=[],
            )
        )

    def _publish_log_progress_events(
        self,
        *,
        request: ExecutionQueueRequest,
        stdout_lines: list[str],
        stderr_lines: list[str],
    ) -> None:
        for line in stdout_lines:
            self._publisher.publish_progress(
                ExecutionProgress(
                    request_id=request.request_id,
                    workflow_id=request.workflow_id,
                    step_id=request.step_id,
                    executor_id=self._executor_id,
                    status="running",
                    progress=0.5,
                    message="stdout",
                    stdout_lines=[line],
                    stderr_lines=[],
                )
            )
        for line in stderr_lines:
            self._publisher.publish_progress(
                ExecutionProgress(
                    request_id=request.request_id,
                    workflow_id=request.workflow_id,
                    step_id=request.step_id,
                    executor_id=self._executor_id,
                    status="running",
                    progress=0.5,
                    message="stderr",
                    stdout_lines=[],
                    stderr_lines=[line],
                )
            )


def _captured_lines(buffer: io.StringIO) -> list[str]:
    return [line for line in buffer.getvalue().splitlines() if line.strip()]
