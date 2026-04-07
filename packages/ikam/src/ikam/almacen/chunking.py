"""Embedding utilities for IKAM v2.

Provides embedding provider protocol and helper functions.
Chunking is handled by forja decomposers — this module retains
only the embedding and token estimation infrastructure.
"""
from __future__ import annotations

import logging
import os
from typing import List, Optional, Protocol, Sequence, Tuple, runtime_checkable


# ---------------------------------------------------------------------------
# EmbeddingProvider protocol — backend-agnostic embedding interface
# ---------------------------------------------------------------------------

@runtime_checkable
class EmbeddingProvider(Protocol):
    """Protocol for embedding providers.

    Implementations live in Layer 1 (modelado) or test fakes.
    ikam Layer 0 never imports openai directly.
    """

    def embed(self, texts: list[str]) -> list[list[float] | None]: ...

logger = logging.getLogger("ikam.almacen.chunking")

EMBED_BATCH_SIZE = int(os.getenv("ANALYSIS_EMBED_BATCH", "16"))
EMBED_MODEL = os.getenv("OPENAI_EMBED_MODEL", "text-embedding-3-small")


def _estimate_tokens(text: str) -> int:
    """Estimate token count for text using word-based heuristic.

    Approximates GPT tokenization: ~1.1 tokens per word.
    Monotonic guarantee: longer text → more tokens.
    """
    words = text.split()
    if not words:
        return max(1, len(text) // 4)
    # Approximate tokens ~ words * 1.1
    return max(1, int(len(words) * 1.1))


def format_vector(vec: Optional[Sequence[float]]) -> Optional[str]:
    """Format float vector for storage (10-digit precision)."""
    if vec is None:
        return None
    return "[" + ",".join(f"{x:.10f}" for x in vec) + "]"


def embed_texts(
    texts: Sequence[str],
    *,
    provider: EmbeddingProvider | None = None,
) -> Tuple[List[Optional[List[float]]], Optional[int]]:
    """Generate embeddings for text chunks via an EmbeddingProvider.

    Args:
        texts: Sequence of text strings to embed
        provider: Optional EmbeddingProvider implementation. When absent,
                  returns ``[None, ...]`` gracefully — no OpenAI import.

    Returns:
        Tuple of (embeddings list, embedding dimension).
        Each embedding is a list of floats or None if no provider.
    """
    if not texts:
        return [], None
    if provider is None:
        return [None for _ in texts], None

    raw = provider.embed(list(texts))
    vectors: List[Optional[List[float]]] = list(raw)
    dimension: Optional[int] = None
    for v in vectors:
        if v is not None:
            dimension = len(v)
            break
    return vectors, dimension


__all__ = [
    "EmbeddingProvider",
    "embed_texts",
    "format_vector",
]
