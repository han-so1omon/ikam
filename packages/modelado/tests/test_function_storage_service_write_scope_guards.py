from __future__ import annotations

from datetime import UTC, datetime

import pytest

from modelado.core.execution_context import (
    ExecutionContext,
    ExecutionMode,
    ExecutionPolicyViolation,
    WriteScope,
    execution_context,
    execution_write_scope,
)
from modelado.core.function_storage import GeneratedFunctionMetadata, GeneratedFunctionRecord
from modelado.core.function_storage_service import FunctionStorageService


@pytest.fixture
def storage(mocker) -> FunctionStorageService:
    service = FunctionStorageService(connection_string=None)
    # Force postgres-like behavior without creating a real ConnectionPool.
    service._storage_mode = "postgres"
    service.connection_pool = mocker.MagicMock()
    return service


def _write_scope() -> WriteScope:
    return WriteScope(
        allowed=True,
        project_id="proj-test",
        operation="unit-test",
        nonce="nonce-1",
        payload_hash="sha256:" + ("0" * 64),
        agent_id="agent-test",
        key_fingerprint="blake3:test",
        signature="ed25519:test",
    )


def _record() -> GeneratedFunctionRecord:
    meta = GeneratedFunctionMetadata(
        user_intent="do thing",
        semantic_intent="do_thing",
        confidence=1.0,
        strategy="unit-test",
        generator_version="test",
        generation_timestamp=datetime.now(UTC),
    )
    return GeneratedFunctionRecord(
        function_id="gfn_test",
        content_hash="h" * 64,
        canonical_code="def f():\n    return 1\n",
        original_code="def f():\n    return 1\n",
        metadata=meta,
        stored_at=datetime.now(UTC),
        storage_key="gfn_test",
        deduplicated=False,
        cache_key=None,
        execution_count=1,
    )


def _mock_db(storage: FunctionStorageService):
    cx = storage.connection_pool.connection.return_value.__enter__.return_value
    cur = cx.cursor.return_value.__enter__.return_value
    return cx, cur


def test_update_record_requires_execution_context(storage):
    cx, cur = _mock_db(storage)

    # packages/modelado/tests/conftest.py installs an autouse execution_context;
    # explicitly clear it to verify enforcement.
    import modelado.core.execution_context as ec

    token = ec._CURRENT.set(None)  # type: ignore[attr-defined]
    try:
        with pytest.raises(ExecutionPolicyViolation):
            import asyncio

            asyncio.run(storage._update_record(_record()))
    finally:
        ec._CURRENT.reset(token)  # type: ignore[attr-defined]

    storage.connection_pool.connection.assert_not_called()
    cx.cursor.assert_not_called()
    cur.execute.assert_not_called()


def test_update_record_blocks_system_actor_without_scope(storage):
    cx, cur = _mock_db(storage)

    with execution_context(ExecutionContext(mode=ExecutionMode.REQUEST, request_id="t", actor_id=None)):
        with pytest.raises(ExecutionPolicyViolation):
            import asyncio

            asyncio.run(storage._update_record(_record()))

    storage.connection_pool.connection.assert_not_called()
    cx.cursor.assert_not_called()
    cur.execute.assert_not_called()


def test_update_record_allows_system_actor_with_scope(storage):
    cx, cur = _mock_db(storage)

    with execution_context(ExecutionContext(mode=ExecutionMode.REQUEST, request_id="t", actor_id=None)):
        with execution_write_scope(_write_scope()):
            import asyncio

            asyncio.run(storage._update_record(_record()))

    storage.connection_pool.connection.assert_called_once()
    assert cur.execute.call_count == 1
    cx.commit.assert_called_once()
