"""Tests for EmbeddingProvider protocol in ikam.almacen.chunking.

Verifies that embed_texts accepts an optional provider argument
and that ikam Layer 0 does not import openai directly.
"""
from __future__ import annotations

from ikam.almacen.chunking import EmbeddingProvider, embed_texts


class FakeProvider:
    """Implements EmbeddingProvider protocol without openai."""

    def embed(self, texts: list[str]) -> list[list[float] | None]:
        return [[0.1, 0.2] for _ in texts]


def test_embed_texts_uses_provider_without_openai_import():
    vectors, dim = embed_texts(["a", "b"], provider=FakeProvider())
    assert vectors == [[0.1, 0.2], [0.1, 0.2]]
    assert dim == 2


def test_embed_texts_returns_none_vectors_when_provider_missing():
    vectors, dim = embed_texts(["a"])  # no provider
    assert vectors == [None]
    assert dim is None
