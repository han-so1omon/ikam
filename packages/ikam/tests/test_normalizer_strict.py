# packages/ikam/tests/test_normalizer_strict.py
from __future__ import annotations

import asyncio
import pytest

from ikam.forja.normalizer import SemanticNormalizer, NormalizationError
from ikam.fragments import Fragment


def test_normalizer_without_llm_raises_type_error():
    """SemanticNormalizer() without ai_client must raise TypeError."""
    with pytest.raises(TypeError):
        SemanticNormalizer()


def test_normalizer_propagates_llm_json_error():
    """If LLM returns malformed JSON, NormalizationError is raised (not silent [])."""

    class _BadLLM:
        async def call_model(self, prompt: str, model: str, temperature: float):
            class R:
                output = "not valid json"
            return R()

    normalizer = SemanticNormalizer(ai_client=_BadLLM())
    source = Fragment(value={"text": "Revenue grew significantly."}, mime_type="text/plain")

    with pytest.raises(NormalizationError, match="LLM"):
        asyncio.run(normalizer.normalize(source))
