"""OpenAI-backed embedding provider implementing EmbeddingProvider protocol."""
from __future__ import annotations

from typing import Any

from modelado.oraculo.llm_trace import emit_llm_trace


class OpenAIEmbeddingProvider:
    """EmbeddingProvider implementation using OpenAI embeddings API.

    Satisfies the runtime-checkable EmbeddingProvider protocol
    defined in ikam.almacen.chunking. Client is created lazily on
    first use to allow construction without OPENAI_API_KEY.
    """

    def __init__(self, model: str = "text-embedding-3-small"):
        self._client: Any = None
        self._model = model

    def _get_client(self) -> Any:
        if self._client is None:
            from openai import OpenAI
            self._client = OpenAI()
        return self._client

    def embed(self, texts: list[str]) -> list[list[float] | None]:
        """Embed texts using OpenAI embeddings API."""
        if not texts:
            return []

        emit_llm_trace(
            provider="openai",
            operation="embeddings.create",
            model=self._model,
            phase="request",
            request_payload={"input_count": len(texts), "inputs": texts[:3]},
            metadata={"component": "OpenAIEmbeddingProvider"},
        )

        try:
            response = self._get_client().embeddings.create(
                model=self._model,
                input=texts,
            )
        except Exception as exc:
            emit_llm_trace(
                provider="openai",
                operation="embeddings.create",
                model=self._model,
                phase="error",
                metadata={"component": "OpenAIEmbeddingProvider", "error": str(exc)},
            )
            raise

        emit_llm_trace(
            provider="openai",
            operation="embeddings.create",
            model=self._model,
            phase="response",
            response_payload={"embedding_count": len(getattr(response, "data", []))},
            metadata={"component": "OpenAIEmbeddingProvider"},
        )

        return [item.embedding for item in response.data]
