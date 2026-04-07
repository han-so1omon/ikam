import pytest
import modelado.ikam_graph_repository_async as async_repository

from ikam.graph import StoredFragment
from modelado.core.execution_context import (
    ExecutionContext,
    ExecutionMode,
    ExecutionPolicyViolation,
    WriteScope,
    execution_context,
    execution_write_scope,
)
from modelado.ikam_graph_repository import insert_fragment


def test_insert_fragment_allows_non_system_actor(db_connection):
    frag = StoredFragment.from_bytes(b"hello", mime_type="text/plain")
    insert_fragment(db_connection, frag)


def test_insert_fragment_blocks_system_actor_without_write_scope(db_connection):
    frag = StoredFragment.from_bytes(b"hello", mime_type="text/plain")

    with execution_context(
        ExecutionContext(mode=ExecutionMode.REQUEST, request_id="t1", actor_id=None, purpose="test")
    ):
        with pytest.raises(ExecutionPolicyViolation):
            insert_fragment(db_connection, frag)


def test_insert_fragment_allows_system_actor_with_write_scope(db_connection):
    frag = StoredFragment.from_bytes(b"hello", mime_type="text/plain")

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
        ExecutionContext(mode=ExecutionMode.REQUEST, request_id="t2", actor_id=None, purpose="test")
    ):
        with execution_write_scope(scope):
            insert_fragment(db_connection, frag)


def test_async_repository_does_not_import_sync_private_helpers():
    assert not hasattr(async_repository, "_generate_cas_bytes")
    assert not hasattr(async_repository, "_cas_id_for_domain_fragment")
    assert not hasattr(async_repository, "_build_domain_id_to_cas_id_map")
