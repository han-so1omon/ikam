import pytest

from modelado.core.execution_context import (
    ExecutionContext,
    ExecutionMode,
    ExecutionPolicyViolation,
    WriteScope,
    execution_context,
    execution_write_scope,
)


def test_artifact_store_blocks_system_actor_without_write_scope(db_connection):
    from modelado.ikam_artifact_store_pg import PostgresArtifactStore

    store = PostgresArtifactStore(db_connection)

    with execution_context(
        ExecutionContext(mode=ExecutionMode.REQUEST, request_id="t-artifact-store", actor_id=None, purpose="test")
    ):
        with pytest.raises(ExecutionPolicyViolation):
            store.upsert_artifact_with_fragments(
                artifact_id="00000000-0000-0000-0000-000000000000",
                kind="document",
                title=None,
                created_at=None,
                fragment_ids=[],
            )


def test_artifact_store_allows_system_actor_with_write_scope(db_connection):
    from modelado.ikam_artifact_store_pg import PostgresArtifactStore

    store = PostgresArtifactStore(db_connection)

    scope = WriteScope(
        allowed=True,
        project_id="proj-test",
        operation="unit-test",
        nonce="nonce-1",
        payload_hash="sha256:" + ("0" * 64),
        agent_id="agent-test",
        key_fingerprint="blake3:test",
        signature="ed25519:test",
    )

    with execution_context(
        ExecutionContext(mode=ExecutionMode.REQUEST, request_id="t-artifact-store", actor_id=None, purpose="test")
    ):
        with execution_write_scope(scope):
            store.upsert_artifact_with_fragments(
                artifact_id="00000000-0000-0000-0000-000000000000",
                kind="document",
                title=None,
                created_at=None,
                fragment_ids=[],
            )
