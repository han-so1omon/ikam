"""Base provider exports for unified LLM client."""

from __future__ import annotations

from modelado.oraculo.ai_client import (
    EmbedRequest,
    EmbedResponse,
    GenerateRequest,
    GenerateResponse,
    JudgeRequest,
    JudgeResponse,
    AIClient,
)

__all__ = [
    "AIClient",
    "GenerateRequest",
    "GenerateResponse",
    "EmbedRequest",
    "EmbedResponse",
    "JudgeRequest",
    "JudgeResponse",
]
