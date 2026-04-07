import pytest

from modelado.core.execution_context import (
    ExecutionContext,
    ExecutionMode,
    ExecutionPolicyViolation,
    WriteScope,
    execution_context,
    execution_write_scope,
    require_write_scope,
)


def test_require_write_scope_requires_execution_context() -> None:
    with pytest.raises(ExecutionPolicyViolation):
        require_write_scope("test-op")


def test_require_write_scope_requires_scope() -> None:
    with execution_context(ExecutionContext(mode=ExecutionMode.REQUEST, request_id="r1")):
        with pytest.raises(ExecutionPolicyViolation):
            require_write_scope("test-op")


def test_require_write_scope_denied_scope() -> None:
    scope = WriteScope(allowed=False, project_id="p", operation="op")
    with execution_context(ExecutionContext(mode=ExecutionMode.REQUEST, request_id="r1", write_scope=scope)):
        with pytest.raises(ExecutionPolicyViolation):
            require_write_scope("test-op")


def test_require_write_scope_allowed_scope_returns_scope() -> None:
    scope = WriteScope(allowed=True, project_id="p", operation="op", nonce="n")
    with execution_context(ExecutionContext(mode=ExecutionMode.REQUEST, request_id="r1", write_scope=scope)):
        got = require_write_scope("test-op")
    assert got is scope


def test_execution_write_scope_requires_active_execution_context() -> None:
    import modelado.core.execution_context as ec

    # packages/modelado/tests/conftest.py installs an autouse execution_context;
    # explicitly clear it here to validate the guard.
    token = ec._CURRENT.set(None)  # type: ignore[attr-defined]
    try:
        scope = WriteScope(allowed=True, project_id="p", operation="op")
        with pytest.raises(ExecutionPolicyViolation):
            with execution_write_scope(scope):
                pass
    finally:
        ec._CURRENT.reset(token)  # type: ignore[attr-defined]


def test_execution_write_scope_sets_scope() -> None:
    scope = WriteScope(allowed=True, project_id="p", operation="op")
    with execution_context(ExecutionContext(mode=ExecutionMode.REQUEST, request_id="r1")):
        with execution_write_scope(scope):
            got = require_write_scope("test-op")
    assert got is scope
