"""modelado.oraculo — Infrastructure adapters for ikam.oraculo protocols.

Layer 1 package: provides OpenAI and Postgres-backed implementations
of JudgeProtocol, EmbeddingProvider, and GraphState.
"""
from __future__ import annotations

__all__ = [
    "AIClient",
    "GenerateRequest",
    "GenerateResponse",
    "EmbedRequest",
    "EmbedResponse",
    "JudgeRequest",
    "JudgeResponse",
    "OpenAIJudge",
    "OpenAIEmbeddingProvider",
    "PersistentGraphState",
]

# Lazy imports to avoid import-time OpenAI dependency
def __getattr__(name: str):
    if name in {
        "AIClient",
        "GenerateRequest",
        "GenerateResponse",
        "EmbedRequest",
        "EmbedResponse",
        "JudgeRequest",
        "JudgeResponse",
    }:
        from .ai_client import (
            AIClient,
            GenerateRequest,
            GenerateResponse,
            EmbedRequest,
            EmbedResponse,
            JudgeRequest,
            JudgeResponse,
        )

        table = {
            "AIClient": AIClient,
            "GenerateRequest": GenerateRequest,
            "GenerateResponse": GenerateResponse,
            "EmbedRequest": EmbedRequest,
            "EmbedResponse": EmbedResponse,
            "JudgeRequest": JudgeRequest,
            "JudgeResponse": JudgeResponse,
        }
        return table[name]
    if name == "OpenAIJudge":
        from .openai_judge import OpenAIJudge
        return OpenAIJudge
    if name == "OpenAIEmbeddingProvider":
        from .openai_embeddings import OpenAIEmbeddingProvider
        return OpenAIEmbeddingProvider
    if name == "PersistentGraphState":
        from .persistent_graph_state import PersistentGraphState
        return PersistentGraphState
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
