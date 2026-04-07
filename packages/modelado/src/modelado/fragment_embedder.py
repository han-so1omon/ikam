"""LocalFragmentEmbedder — EmbeddingGemma-backed fragment embedder.

Default model: google/embeddinggemma-300m (768d, Gemma license).
Set IKAM_EMBEDDING_MODEL=all-MiniLM-L6-v2 for the lighter CI/test alternative (384d).
Set HUGGINGFACE_TOKEN for gated model access (required for embeddinggemma-300m).

sentence-transformers is a hard dependency — no fallbacks.

Process-level singleton: use get_shared_embedder() instead of instantiating
LocalFragmentEmbedder directly to avoid reloading the model on each call.
"""
from __future__ import annotations

import os
from typing import Optional

import numpy as np
from sentence_transformers import SentenceTransformer

from ikam.fragments import Fragment
from ikam.ir.text_conversion import fragment_to_text

_DEFAULT_MODEL = "google/embeddinggemma-300m"


class LocalFragmentEmbedder:
    """Local embedding model for fragment dedup.

    Uses sentence-transformers. Default model is google/embeddinggemma-300m (768d).
    """

    def __init__(self, model_name: Optional[str] = None, target_dim: int = 768):
        self._model_name = model_name or os.environ.get("IKAM_EMBEDDING_MODEL", _DEFAULT_MODEL)
        self._target_dim = target_dim
        self._model: SentenceTransformer | None = None
        self._native_dim: int | None = None

    def _load_model(self):
        if self._model is None:
            token = os.environ.get("HUGGINGFACE_TOKEN") or None
            self._model = SentenceTransformer(self._model_name, token=token)
            self._native_dim = self._model.get_sentence_embedding_dimension()

    def sync_embed(self, text: str) -> list[float]:
        """Synchronously embed text."""
        self._load_model()
        if self._model is None:
            raise RuntimeError("Failed to load SentenceTransformer model")
        embedding = self._model.encode(text, convert_to_numpy=True).tolist()
        # Pad to target_dim if needed
        if len(embedding) < self._target_dim:
            embedding.extend([0.0] * (self._target_dim - len(embedding)))
        return embedding[:self._target_dim]

    async def embed(self, fragment: Fragment) -> list[float]:
        self._load_model()
        text = fragment_to_text(fragment)
        return self.sync_embed(text)

    def encode_texts(self, texts: list[str]) -> np.ndarray:
        """Encode a batch of strings and return float32 ndarray of shape (N, dim)."""
        self._load_model()
        if self._model is None:
            raise RuntimeError("Failed to load SentenceTransformer model")
        raw = self._model.encode(texts, convert_to_numpy=True)
        return np.array(raw, dtype=np.float32)

    @property
    def dimensions(self) -> int:
        return self._target_dim

    @property
    def model_name(self) -> str:
        return self._model_name


# --- Process-level singleton ---

_SHARED_EMBEDDER: LocalFragmentEmbedder | None = None
_SHARED_MODEL_NAME: str | None = None


def get_shared_embedder() -> LocalFragmentEmbedder:
    """Return the process-level shared embedder, creating it on first call.

    Recreates the singleton if IKAM_EMBEDDING_MODEL has changed since last call.
    """
    global _SHARED_EMBEDDER, _SHARED_MODEL_NAME
    name = os.environ.get("IKAM_EMBEDDING_MODEL", _DEFAULT_MODEL)
    if _SHARED_EMBEDDER is None or _SHARED_MODEL_NAME != name:
        _SHARED_EMBEDDER = LocalFragmentEmbedder(model_name=name)
        _SHARED_MODEL_NAME = name
    return _SHARED_EMBEDDER
