import pytest

from modelado.core.execution_context import (
    ExecutionContext,
    ExecutionMode,
    ExecutionPolicyViolation,
    WriteScope,
    execution_context,
)


def test_record_system_mutation_event_enforces_scope_project_id_match() -> None:
    from modelado.ikam_graph_repository import record_system_mutation_event

    scope = WriteScope(
        allowed=True,
        project_id="p1",
        operation="op",
        agent_id="agent",
        key_fingerprint="kf",
        signature="ed25519:sig",
        nonce="n",
        payload_hash="sha256:" + "0" * 64,
    )

    with execution_context(ExecutionContext(mode=ExecutionMode.REQUEST, request_id="r1", write_scope=scope)):
        with pytest.raises(ExecutionPolicyViolation):
            record_system_mutation_event(  # type: ignore[arg-type]
                None,
                artifact_id="a",
                operation="op",
                project_id="p2",
                payload=None,
                agent_id="agent",
                key_fingerprint="kf",
                signature="ed25519:sig",
            )


def test_record_system_mutation_event_enforces_scope_operation_match() -> None:
    from modelado.ikam_graph_repository import record_system_mutation_event

    scope = WriteScope(
        allowed=True,
        project_id="p",
        operation="op1",
        agent_id="agent",
        key_fingerprint="kf",
        signature="ed25519:sig",
        nonce="n",
        payload_hash="sha256:" + "0" * 64,
    )

    with execution_context(ExecutionContext(mode=ExecutionMode.REQUEST, request_id="r1", write_scope=scope)):
        with pytest.raises(ExecutionPolicyViolation):
            record_system_mutation_event(  # type: ignore[arg-type]
                None,
                artifact_id="a",
                operation="op2",
                project_id="p",
                payload=None,
                agent_id="agent",
                key_fingerprint="kf",
                signature="ed25519:sig",
            )


def test_record_system_mutation_event_enforces_scope_agent_id_match() -> None:
    from modelado.ikam_graph_repository import record_system_mutation_event

    scope = WriteScope(
        allowed=True,
        project_id="p",
        operation="op",
        agent_id="agent-1",
        key_fingerprint="kf",
        signature="ed25519:sig",
        nonce="n",
        payload_hash="sha256:" + "0" * 64,
    )

    with execution_context(ExecutionContext(mode=ExecutionMode.REQUEST, request_id="r1", write_scope=scope)):
        with pytest.raises(ExecutionPolicyViolation):
            record_system_mutation_event(  # type: ignore[arg-type]
                None,
                artifact_id="a",
                operation="op",
                project_id="p",
                payload=None,
                agent_id="agent-2",
                key_fingerprint="kf",
                signature="ed25519:sig",
            )
