from __future__ import annotations

from contextlib import contextmanager
from contextvars import ContextVar
from dataclasses import dataclass, replace
from enum import Enum
from typing import Iterator, Optional


@dataclass(frozen=True)
class WriteScope:
    """Authorization-bearing scope required for IKAM write operations.

    This is intentionally DB-independent. It can be constructed from a
    verified signed envelope and propagated through an ExecutionContext.
    """

    allowed: bool
    project_id: str
    operation: str
    nonce: Optional[str] = None
    payload_hash: Optional[str] = None
    agent_id: Optional[str] = None
    key_fingerprint: Optional[str] = None
    signature: Optional[str] = None


class ExecutionMode(str, Enum):
    REQUEST = "request"
    BACKGROUND = "background"


@dataclass(frozen=True)
class ExecutionContext:
    mode: ExecutionMode
    request_id: Optional[str] = None
    actor_id: Optional[str] = None
    purpose: Optional[str] = None
    write_scope: Optional[WriteScope] = None


class ExecutionPolicyViolation(RuntimeError):
    pass


_CURRENT: ContextVar[Optional[ExecutionContext]] = ContextVar(
    "modelado_execution_context", default=None
)


def get_execution_context() -> Optional[ExecutionContext]:
    return _CURRENT.get()


@contextmanager
def execution_context(ctx: ExecutionContext) -> Iterator[ExecutionContext]:
    token = _CURRENT.set(ctx)
    try:
        yield ctx
    finally:
        _CURRENT.reset(token)


@contextmanager
def execution_write_scope(scope: WriteScope) -> Iterator[WriteScope]:
    """Temporarily attach a WriteScope to the current ExecutionContext.

    This is useful for enforcing write guards deep in repo code, without
    threading authz details through every call site.
    """

    ctx = get_execution_context()
    if ctx is None:
        raise ExecutionPolicyViolation(
            "Execution policy violation: cannot set write scope without an active execution context"
        )
    with execution_context(replace(ctx, write_scope=scope)):
        yield scope


def require_request_scope(operation: str) -> None:
    ctx = get_execution_context()
    if ctx is None or ctx.mode != ExecutionMode.REQUEST:
        raise ExecutionPolicyViolation(
            f"Execution policy violation: '{operation}' requires request scope "
            f"(ctx={ctx})"
        )


def require_write_scope(operation: str) -> WriteScope:
    ctx = get_execution_context()
    scope = ctx.write_scope if ctx else None
    if scope is None:
        raise ExecutionPolicyViolation(
            f"Execution policy violation: '{operation}' requires write scope "
            f"(ctx={ctx})"
        )
    if not scope.allowed:
        raise ExecutionPolicyViolation(
            f"Execution policy violation: '{operation}' denied by write scope "
            f"(ctx={ctx})"
        )
    return scope
