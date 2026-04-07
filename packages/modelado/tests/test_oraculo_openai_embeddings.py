"""Tests for OpenAIEmbeddingProvider — modelado adapter implementing EmbeddingProvider."""
from __future__ import annotations

import os

import pytest

from ikam.almacen.chunking import EmbeddingProvider


needs_openai = pytest.mark.skipif(
    not os.environ.get("OPENAI_API_KEY"),
    reason="OPENAI_API_KEY not set",
)


def test_openai_embedding_provider_satisfies_protocol():
    """OpenAIEmbeddingProvider must be a runtime-checkable EmbeddingProvider."""
    from modelado.oraculo.openai_embeddings import OpenAIEmbeddingProvider

    provider = OpenAIEmbeddingProvider(model="text-embedding-3-small")
    assert isinstance(provider, EmbeddingProvider)


def test_openai_embedding_provider_embed_with_mock(monkeypatch):
    """Verify OpenAIEmbeddingProvider returns list of vectors from mock."""
    from modelado.oraculo.openai_embeddings import OpenAIEmbeddingProvider

    class FakeEmbeddingData:
        def __init__(self, vec):
            self.embedding = vec

    class FakeEmbeddingResponse:
        def __init__(self, vecs):
            self.data = [FakeEmbeddingData(v) for v in vecs]

    class FakeEmbeddings:
        @staticmethod
        def create(**kwargs):
            texts = kwargs.get("input", [])
            return FakeEmbeddingResponse([[0.1, 0.2, 0.3] for _ in texts])

    class FakeClient:
        embeddings = FakeEmbeddings()

    provider = OpenAIEmbeddingProvider(model="text-embedding-3-small")
    provider._client = FakeClient()

    vectors = provider.embed(["hello", "world"])
    assert len(vectors) == 2
    assert vectors[0] == [0.1, 0.2, 0.3]
    assert vectors[1] == [0.1, 0.2, 0.3]


@needs_openai
def test_openai_embedding_provider_live_call():
    """Integration test: verify real OpenAI embedding returns valid vectors."""
    from modelado.oraculo.openai_embeddings import OpenAIEmbeddingProvider

    provider = OpenAIEmbeddingProvider(model="text-embedding-3-small")
    vectors = provider.embed(["hello world"])
    assert len(vectors) == 1
    assert isinstance(vectors[0], list)
    assert len(vectors[0]) > 0
    assert all(isinstance(v, float) for v in vectors[0])
