"""Unified provider-agnostic AI client contracts.

This module defines a single interface for generation, embeddings, and judging.
Provider adapters implement this protocol, while runtime modules depend only on
these contracts.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Protocol, runtime_checkable


@dataclass(frozen=True)
class GenerateRequest:
    messages: list[dict[str, str]]
    model: str
    max_tokens: int | None = None
    temperature: float = 0.0
    seed: int | None = None
    response_format: dict[str, Any] | None = None
    metadata: dict[str, Any] = field(default_factory=dict)


@dataclass(frozen=True)
class GenerateResponse:
    text: str
    provider: str
    model: str
    usage: dict[str, int] = field(default_factory=dict)
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass(frozen=True)
class EmbedRequest:
    texts: list[str]
    model: str
    metadata: dict[str, Any] = field(default_factory=dict)


@dataclass(frozen=True)
class EmbedResponse:
    vectors: list[list[float]]
    provider: str
    model: str
    usage: dict[str, int] = field(default_factory=dict)
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass(frozen=True)
class JudgeRequest:
    question: str
    context: dict[str, Any] | None = None
    candidates: list[dict[str, Any]] | None = None
    model: str = ""
    metadata: dict[str, Any] = field(default_factory=dict)


@dataclass(frozen=True)
class JudgeResponse:
    score: float
    reasoning: str
    facts_found: list[str] | None
    provider: str
    model: str
    metadata: dict[str, Any] = field(default_factory=dict)
    raw: dict[str, Any] = field(default_factory=dict)


@runtime_checkable
class AIClient(Protocol):
    """Provider-neutral interface for all runtime AI operations."""

    async def generate(self, request: GenerateRequest) -> GenerateResponse:
        ...

    async def embed(self, request: EmbedRequest) -> EmbedResponse:
        ...

    async def judge(self, request: JudgeRequest) -> JudgeResponse:
        ...
