from __future__ import annotations

from datetime import datetime, timezone
from typing import Any
from uuid import uuid4

from interacciones.graph.workflow.bus import PublishedMessage
from interacciones.graph.workflow.dispatcher import WorkflowExecutionDispatcher
from interacciones.graph.workflow.state_store import InMemoryWorkflowStateStore
from interacciones.graph.workflow.state_store_protocol import WorkflowStateStore
from interacciones.graph.workflow.trace_promotion import build_trace_promotion_plan
from interacciones.graph.workflow.trace_promotion_sink import TracePromotionSink
from interacciones.schemas import TracePersistencePolicy
from interacciones.schemas.execution import ApprovalRequested, ApprovalResolved, ExecutionCompleted, ExecutionFailed, ExecutionProgress, ExecutionQueued, ExecutionRequest
from interacciones.schemas.executors import ExecutorDeclaration


class WorkflowOrchestrator:
    def __init__(
        self,
        declarations: list[ExecutorDeclaration],
        *,
        state_store: WorkflowStateStore | None = None,
        trace_store: Any | None = None,
        trace_policy: TracePersistencePolicy | None = None,
        trace_promotion_sink: TracePromotionSink | None = None,
    ) -> None:
        self._dispatcher = WorkflowExecutionDispatcher(declarations)
        self._state_store = state_store or InMemoryWorkflowStateStore()
        self._trace_store = trace_store
        self._trace_policy = trace_policy
        self._trace_promotion_sink = trace_promotion_sink
        self._pending_trace_promotions: list[dict[str, Any]] = []

    def dispatch_execution(self, request: ExecutionRequest):
        published = self._dispatcher.dispatch(request)
        self._state_store.record_dispatch(
            workflow_id=request.workflow_id,
            step_id=request.step_id,
            executor_id=published.payload["executor_id"],
            request_id=request.request_id,
            capability=request.capability,
            policy=request.policy,
            constraints=request.constraints,
            payload=request.payload,
        )
        self._append_trace(
            workflow_id=request.workflow_id,
            step_id=request.step_id,
            event_type="execution.dispatched",
            request_id=request.request_id,
            executor_id=published.payload["executor_id"],
            trace_context=_trace_context_from_payload(request.payload),
            payload=request.payload,
        )
        return published

    def handle_execution_progress(self, event: ExecutionProgress) -> None:
        stdout_lines = list(getattr(event, "stdout_lines", []))
        stderr_lines = list(getattr(event, "stderr_lines", []))
        self._state_store.record_progress(
            workflow_id=event.workflow_id,
            step_id=event.step_id,
            executor_id=event.executor_id,
            status=event.status,
            progress=event.progress,
            message=event.message,
            stdout_lines=stdout_lines,
            stderr_lines=stderr_lines,
        )
        self._append_trace(
            workflow_id=event.workflow_id,
            step_id=event.step_id,
            event_type="execution.progress",
            request_id=event.request_id,
            executor_id=event.executor_id,
            trace_context=self._trace_context_for_event(event.workflow_id, event.details),
            payload={
                "status": event.status,
                "progress": event.progress,
                "message": event.message,
                "stdout_lines": stdout_lines,
                "stderr_lines": stderr_lines,
            },
        )

    def handle_execution_queued(self, event: ExecutionQueued) -> None:
        self._state_store.record_queued(
            workflow_id=event.workflow_id,
            step_id=event.step_id,
            executor_id=event.executor_id,
            executor_kind=event.executor_kind,
            request_id=event.request_id,
            capability=event.capability,
        )
        self._append_trace(
            workflow_id=event.workflow_id,
            step_id=event.step_id,
            event_type="execution.queued",
            request_id=event.request_id,
            executor_id=event.executor_id,
            payload={
                "executor_kind": event.executor_kind,
                "capability": event.capability,
                "status": event.status,
            },
        )

    def handle_execution_completed(self, event: ExecutionCompleted) -> None:
        stdout_lines = list(getattr(event, "stdout_lines", []))
        stderr_lines = list(getattr(event, "stderr_lines", []))
        self._state_store.record_completion(
            workflow_id=event.workflow_id,
            step_id=event.step_id,
            executor_id=event.executor_id,
            result=event.result,
            artifacts=event.artifacts,
            stdout_lines=stdout_lines,
            stderr_lines=stderr_lines,
        )
        self._append_trace(
            workflow_id=event.workflow_id,
            step_id=event.step_id,
            event_type="execution.completed",
            request_id=event.request_id,
            executor_id=event.executor_id,
            payload={
                "result": event.result,
                "artifacts": event.artifacts,
                "stdout_lines": stdout_lines,
                "stderr_lines": stderr_lines,
            },
            trace_context=self._trace_context_for_event(event.workflow_id),
        )

    def handle_execution_failed(self, event: ExecutionFailed) -> None:
        stdout_lines = list(getattr(event, "stdout_lines", []))
        stderr_lines = list(getattr(event, "stderr_lines", []))
        retry_count = event.details.get("retry_count")
        max_retries = event.details.get("max_retries")
        deadline_at = event.details.get("deadline_at")
        if event.retryable and retry_count is not None and max_retries is not None:
            self._state_store.record_retry_metadata(
                workflow_id=event.workflow_id,
                step_id=event.step_id,
                retry_count=int(retry_count),
                max_retries=int(max_retries),
                deadline_at=str(deadline_at) if deadline_at is not None else None,
            )
            self._append_trace(
                workflow_id=event.workflow_id,
                step_id=event.step_id,
                event_type="execution.failed",
                request_id=event.request_id,
                executor_id=event.executor_id,
                trace_context=self._trace_context_for_event(event.workflow_id, event.details),
                payload={
                    "error_code": event.error_code,
                    "error_message": event.error_message,
                    "retryable": event.retryable,
                    **event.details,
                },
            )
            return
        self._state_store.record_failure(
            workflow_id=event.workflow_id,
            step_id=event.step_id,
            executor_id=event.executor_id,
            error_code=event.error_code,
            error_message=event.error_message,
            retryable=event.retryable,
            stdout_lines=stdout_lines,
            stderr_lines=stderr_lines,
        )
        self._append_trace(
            workflow_id=event.workflow_id,
            step_id=event.step_id,
            event_type="execution.failed",
            request_id=event.request_id,
            executor_id=event.executor_id,
            trace_context=self._trace_context_for_event(event.workflow_id, event.details),
            payload={
                "error_code": event.error_code,
                "error_message": event.error_message,
                "retryable": event.retryable,
                "stdout_lines": stdout_lines,
                "stderr_lines": stderr_lines,
            },
        )

    def handle_execution_event(self, event: Any) -> None:
        if isinstance(event, ExecutionQueued):
            self.handle_execution_queued(event)
            return
        if isinstance(event, ExecutionProgress):
            self.handle_execution_progress(event)
            return
        if isinstance(event, ExecutionCompleted):
            self.handle_execution_completed(event)
            return
        if isinstance(event, ExecutionFailed):
            self.handle_execution_failed(event)
            return
        raise ValueError(f"unsupported execution event type: {type(event).__name__}")

    def handle_approval_requested(self, event: ApprovalRequested) -> None:
        self._state_store.record_approval_requested(
            workflow_id=event.workflow_id,
            step_id=event.step_id,
            approval_id=event.approval_id,
            requested_by=event.requested_by,
            summary=event.summary,
            details=event.details,
        )
        self._append_trace(
            workflow_id=event.workflow_id,
            step_id=event.step_id,
            event_type="approval.requested",
            approval_id=event.approval_id,
            payload={"requested_by": event.requested_by, "summary": event.summary, "details": event.details},
            trace_context=self._trace_context_for_event(event.workflow_id, event.details),
        )

    def handle_approval_resolved(self, event: ApprovalResolved) -> None:
        self._state_store.record_approval_resolved(
            workflow_id=event.workflow_id,
            step_id=event.step_id,
            approval_id=event.approval_id,
            resolved_by=event.resolved_by,
            approved=event.approved,
            comment=event.comment,
        )
        self._append_trace(
            workflow_id=event.workflow_id,
            step_id=event.step_id,
            event_type="approval.resolved",
            approval_id=event.approval_id,
            payload={"resolved_by": event.resolved_by, "approved": event.approved, "comment": event.comment},
            trace_context=self._trace_context_for_event(event.workflow_id),
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
        self._state_store.record_retry_metadata(
            workflow_id=workflow_id,
            step_id=step_id,
            retry_count=retry_count,
            max_retries=max_retries,
            deadline_at=deadline_at,
        )

    def handle_scheduler_action(self, action: dict[str, Any]) -> PublishedMessage | None:
        if action.get("kind") != "retry_wakeup":
            raise ValueError(f"unsupported scheduler action kind: {action.get('kind')}")
        state = self._state_store.get(action["workflow_id"])
        if not state:
            return None
        if state.get("status") != "retry_scheduled":
            return None
        if state.get("current_step") != action.get("step_id"):
            return None
        if state.get("lease_owner") != action.get("lease_owner"):
            return None
        self._append_trace(
            workflow_id=action["workflow_id"],
            step_id=action["step_id"],
            event_type="scheduler.retry_wakeup",
            request_id=state.get("request_id"),
            lease_owner=action.get("lease_owner"),
            trace_context=self._trace_context_for_event(action["workflow_id"], state.get("payload")),
            payload={
                "retry_count": action.get("retry_count"),
                "max_retries": action.get("max_retries"),
                "deadline_at": action.get("deadline_at"),
            },
        )
        request = _request_from_state(state)
        if request is None:
            return None
        return self.dispatch_execution(request)

    def _append_trace(
        self,
        *,
        workflow_id: str,
        step_id: str,
        event_type: str,
        request_id: str | None = None,
        executor_id: str | None = None,
        approval_id: str | None = None,
        lease_owner: str | None = None,
        trace_context: dict[str, Any] | None = None,
        payload: dict[str, Any] | None = None,
    ) -> None:
        if self._trace_store is None:
            return
        event = {
            "trace_id": str(uuid4()),
            "workflow_id": workflow_id,
            "run_id": workflow_id,
            "step_id": step_id,
            "event_type": event_type,
            "occurred_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
            "request_id": request_id,
            "executor_id": executor_id,
            "approval_id": approval_id,
            "lease_owner": lease_owner,
            "payload": payload or {},
        }
        event.update(trace_context or {})
        ref = _trace_ref_from_event(event)
        if ref is not None:
            event["ref"] = ref
        self._trace_store.append(event)
        if self._trace_policy is None:
            return
        hot_events = self._trace_store.list_events(workflow_id=workflow_id, run_id=workflow_id)
        promotion = build_trace_promotion_plan(
            self._trace_policy,
            hot_events=hot_events,
            event=event,
            is_final=event_type in {"execution.completed", "execution.failed"},
        )
        if promotion is not None:
            self._pending_trace_promotions.append(promotion)

    def get_pending_trace_promotions(self) -> list[dict[str, Any]]:
        return [dict(item) for item in self._pending_trace_promotions]

    def _trace_context_for_event(self, workflow_id: str, data: dict[str, Any] | None = None) -> dict[str, Any]:
        context = _trace_context_from_mapping(data)
        if "ref" in context:
            return context
        state = self._state_store.get(workflow_id)
        if not state:
            return context
        return {
            **_trace_context_from_payload(state.get("payload")),
            **context,
        }

    def flush_trace_promotions(self) -> list[dict[str, Any]]:
        flushed = [dict(item) for item in self._pending_trace_promotions]
        if self._trace_promotion_sink is not None:
            flushed = []
            for item in self._pending_trace_promotions:
                self._trace_promotion_sink.record(item)
                if hasattr(self._trace_promotion_sink, "records"):
                    records = getattr(self._trace_promotion_sink, "records")
                    flushed.append(dict(records[-1]))
                else:
                    flushed.append(dict(item))
        self._pending_trace_promotions.clear()
        return flushed


def _request_from_state(state: dict[str, Any]) -> ExecutionRequest | None:
    required = ("request_id", "workflow_id", "current_step", "capability", "policy", "constraints", "payload")
    if any(key not in state for key in required):
        return None
    return ExecutionRequest(
        request_id=state["request_id"],
        workflow_id=state["workflow_id"],
        step_id=state["current_step"],
        capability=state["capability"],
        policy=state["policy"],
        constraints=state["constraints"],
        payload=state["payload"],
    )


def _trace_context_from_payload(payload: dict[str, Any] | None) -> dict[str, Any]:
    if not isinstance(payload, dict):
        return {}
    if isinstance(payload.get("ref"), str):
        return {"ref": payload["ref"]}
    raw = payload.get("_trace")
    if not isinstance(raw, dict):
        return {}
    return _trace_context_from_mapping(raw)


def _trace_context_from_mapping(data: dict[str, Any] | None) -> dict[str, Any]:
    if not isinstance(data, dict):
        return {}
    context: dict[str, Any] = {}
    for key in ("ref", "transition_id", "marking_before_ref", "marking_after_ref", "enabled_transition_ids"):
        value = data.get(key)
        if value is not None:
            context[key] = value
    return context


def _trace_ref_from_event(event: dict[str, Any]) -> str | None:
    direct_ref = event.get("ref")
    if isinstance(direct_ref, str) and direct_ref:
        return direct_ref
    payload = event.get("payload")
    if isinstance(payload, dict):
        payload_ref = payload.get("ref")
        if isinstance(payload_ref, str) and payload_ref:
            return payload_ref
    return None
