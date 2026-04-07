"""Contract tests for execution and approval schemas."""

import pytest
from pydantic import ValidationError

from interacciones.schemas import (
    ApprovalRequested,
    ApprovalResolved,
    ExecutionCompleted,
    ExecutionFailed,
    ExecutionProgress,
    ExecutionQueued,
    ExecutionQueueRequest,
    ExecutionRequest,
    ExecutionScope,
    ResolutionMode,
)


def test_execution_request_round_trip() -> None:
    request = ExecutionRequest(
        request_id="req-1",
        workflow_id="wf-1",
        step_id="step-1",
        capability="python.chunk",
        policy={"latency_tier": "interactive"},
        constraints={"max_items": 100},
        payload={"artifact_id": "art-1"},
    )

    dumped = request.model_dump(mode="json")

    assert dumped == {
        "request_id": "req-1",
        "workflow_id": "wf-1",
        "step_id": "step-1",
        "capability": "python.chunk",
        "policy": {"latency_tier": "interactive"},
        "constraints": {"max_items": 100},
        "payload": {"artifact_id": "art-1"},
        "resolution_mode": "capability_policy",
        "direct_executor_ref": None,
    }


def test_execution_scope_accepts_branch_ref() -> None:
    scope = ExecutionScope(ref=" refs/heads/main ")

    assert scope.model_dump(mode="json") == {"ref": "refs/heads/main"}


def test_execution_scope_rejects_invalid_branch_ref() -> None:
    with pytest.raises(ValidationError):
        ExecutionScope(ref="refs/heads/")


def test_execution_request_supports_direct_executor_resolution() -> None:
    request = ExecutionRequest(
        request_id="req-2",
        workflow_id="wf-1",
        step_id="step-2",
        capability="ml.embed",
        policy={},
        constraints={},
        payload={"text": "hola"},
        resolution_mode=ResolutionMode.DIRECT_EXECUTOR_REF,
        direct_executor_ref="executor://ml-primary",
    )

    assert request.resolution_mode == ResolutionMode.DIRECT_EXECUTOR_REF
    assert request.direct_executor_ref == "executor://ml-primary"


def test_execution_request_requires_direct_executor_ref_for_direct_resolution() -> None:
    with pytest.raises(ValidationError):
        ExecutionRequest(
            request_id="req-2b",
            workflow_id="wf-1",
            step_id="step-2",
            capability="ml.embed",
            policy={},
            constraints={},
            payload={"text": "hola"},
            resolution_mode=ResolutionMode.DIRECT_EXECUTOR_REF,
        )


def test_execution_request_rejects_blank_direct_executor_ref() -> None:
    with pytest.raises(ValidationError):
        ExecutionRequest(
            request_id="req-2c",
            workflow_id="wf-1",
            step_id="step-2",
            capability="ml.embed",
            policy={},
            constraints={},
            payload={"text": "hola"},
            resolution_mode=ResolutionMode.DIRECT_EXECUTOR_REF,
            direct_executor_ref="   ",
        )


def test_execution_request_rejects_unknown_resolution_mode() -> None:
    with pytest.raises(ValidationError):
        ExecutionRequest.model_validate(
            {
                "request_id": "req-3",
                "workflow_id": "wf-1",
                "step_id": "step-3",
                "capability": "ml.embed",
                "policy": {},
                "constraints": {},
                "payload": {},
                "resolution_mode": "invalid",
            }
        )


def test_execution_request_requires_policy_constraints_and_payload() -> None:
    with pytest.raises(ValidationError):
        ExecutionRequest.model_validate(
            {
                "request_id": "req-4",
                "workflow_id": "wf-1",
                "step_id": "step-4",
                "capability": "python.fetch",
            }
        )


def test_execution_queue_request_round_trip() -> None:
    request = ExecutionQueueRequest(
        request_id="req-q-1",
        workflow_id="wf-1",
        step_id="step-1",
        executor_id="executor://python-primary",
        executor_kind="python-executor",
        capability="python.chunk",
        policy={"latency_tier": "interactive"},
        constraints={"max_items": 100},
        payload={"artifact_id": "art-1"},
        transport={"kind": "redpanda", "request_topic": "execution.requests"},
    )

    assert request.model_dump(mode="json") == {
        "request_id": "req-q-1",
        "workflow_id": "wf-1",
        "step_id": "step-1",
        "executor_id": "executor://python-primary",
        "executor_kind": "python-executor",
        "capability": "python.chunk",
        "policy": {"latency_tier": "interactive"},
        "constraints": {"max_items": 100},
        "payload": {"artifact_id": "art-1"},
        "transport": {"kind": "redpanda", "request_topic": "execution.requests"},
    }


def test_execution_progress_completed_and_failed_contracts() -> None:
    progress = ExecutionProgress(
        request_id="req-1",
        workflow_id="wf-1",
        step_id="step-1",
        executor_id="python-executor",
        status="running",
        progress=0.5,
        message="Halfway",
        details={"processed": 50},
    )
    completed = ExecutionCompleted(
        request_id="req-1",
        workflow_id="wf-1",
        step_id="step-1",
        executor_id="python-executor",
        result={"chunks": 4},
        artifacts=["fragment://abc"],
    )
    failed = ExecutionFailed(
        request_id="req-1",
        workflow_id="wf-1",
        step_id="step-1",
        executor_id="python-executor",
        error_code="timeout",
        error_message="Executor timed out",
        retryable=True,
        details={"timeout_seconds": 30},
    )

    assert progress.model_dump(mode="json") == {
        "request_id": "req-1",
        "workflow_id": "wf-1",
        "step_id": "step-1",
        "executor_id": "python-executor",
        "status": "running",
        "progress": 0.5,
        "message": "Halfway",
        "details": {"processed": 50},
    }
    assert completed.model_dump(mode="json") == {
        "request_id": "req-1",
        "workflow_id": "wf-1",
        "step_id": "step-1",
        "executor_id": "python-executor",
        "result": {"chunks": 4},
        "artifacts": ["fragment://abc"],
    }
    assert failed.model_dump(mode="json") == {
        "request_id": "req-1",
        "workflow_id": "wf-1",
        "step_id": "step-1",
        "executor_id": "python-executor",
        "error_code": "timeout",
        "error_message": "Executor timed out",
        "retryable": True,
        "details": {"timeout_seconds": 30},
    }


def test_execution_queued_contract_round_trip() -> None:
    queued = ExecutionQueued(
        request_id="req-queued-1",
        workflow_id="wf-queued",
        step_id="dispatch-parse",
        executor_id="executor://python-primary",
        executor_kind="python-executor",
        capability="python.parse_artifacts",
        status="queued",
    )

    assert queued.model_dump(mode="json") == {
        "request_id": "req-queued-1",
        "workflow_id": "wf-queued",
        "step_id": "dispatch-parse",
        "executor_id": "executor://python-primary",
        "executor_kind": "python-executor",
        "capability": "python.parse_artifacts",
        "status": "queued",
    }


def test_approval_contracts_round_trip() -> None:
    requested = ApprovalRequested(
        approval_id="approval-1",
        workflow_id="wf-1",
        step_id="approve-step",
        requested_by="orchestrator",
        summary="Approve ingestion",
        details={"reason": "manual checkpoint"},
    )
    resolved = ApprovalResolved(
        approval_id="approval-1",
        workflow_id="wf-1",
        step_id="approve-step",
        resolved_by="human:user-1",
        approved=True,
        comment="Looks good",
    )

    assert requested.model_dump(mode="json")["summary"] == "Approve ingestion"
    assert resolved.model_dump(mode="json")["approved"] is True
