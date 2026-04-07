"""Deliberation sink interface for modelado routing summaries.

Defines a transport-agnostic hook to emit structured deliberation breadcrumbs
without importing Kafka or base-api services.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Optional, Protocol, Sequence, Mapping, Any


@dataclass(frozen=True)
class DeliberationContext:
    """Context for a deliberation emission."""

    project_id: str
    session_id: Optional[str]
    parent_id: Optional[str]
    run_id: str


@dataclass(frozen=True)
class DeliberationEnvelopePayload:
    """Payload compatible with interacciones deliberation envelopes."""

    run_id: str
    project_id: str
    ts: int
    phase: str
    status: str
    summary: str
    details: Optional[str] = None
    evidence: Optional[Sequence[Mapping[str, Any]]] = None


class DeliberationSink(Protocol):
    """Interface for emitting deliberation breadcrumbs."""

    def emit(
        self,
        *,
        envelope: DeliberationEnvelopePayload,
        context: DeliberationContext,
    ) -> None:
        """Emit a deliberation envelope with routing context."""


class NoOpDeliberationSink:
    """Default sink that drops all deliberation emissions."""

    def emit(
        self,
        *,
        envelope: DeliberationEnvelopePayload,
        context: DeliberationContext,
    ) -> None:
        return
