"""JudgeProtocol — backend-agnostic LLM-as-judge contract.

Defines the protocol that any judge implementation (OpenAI, Anthropic,
local model, or test stub) must satisfy, plus the query/response types.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Protocol, runtime_checkable


@runtime_checkable
class JudgeProtocol(Protocol):
    """Protocol for LLM-as-judge implementations."""

    def judge(self, query: JudgeQuery) -> Judgment: ...


@dataclass
class JudgeQuery:
    """A question posed to the judge with supporting context."""

    question: str
    context: dict[str, Any] = field(default_factory=dict)
    candidates: list[Any] | None = None


@dataclass
class Judgment:
    """The judge's response: a score, reasoning, and optional metadata."""

    score: float  # 0.0 – 1.0
    reasoning: str
    facts_found: list[str] | None = None
    metadata: dict[str, Any] = field(default_factory=dict)


__all__ = [
    "JudgeProtocol",
    "JudgeQuery",
    "Judgment",
]
