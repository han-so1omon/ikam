# packages/ikam/tests/test_enricher_strict.py
from __future__ import annotations

import asyncio
import json

import pytest

from ikam.forja.enricher import EntityRelationEnricher, EnrichmentError
from ikam.fragments import Fragment


def test_enricher_without_llm_raises_type_error():
    """EntityRelationEnricher() without ai_client must raise TypeError."""
    with pytest.raises(TypeError):
        EntityRelationEnricher()


def test_enricher_propagates_llm_json_error():
    """If LLM returns malformed JSON, EnrichmentError is raised (not silent [])."""

    class _BadLLM:
        async def call_model(self, prompt: str, model: str, temperature: float):
            class R:
                output = "not json at all {{{"
            return R()

    enricher = EntityRelationEnricher(ai_client=_BadLLM())
    source = Fragment(value={"text": "Acme Corp revenue grew."}, mime_type="text/plain")

    with pytest.raises(EnrichmentError, match="LLM"):
        asyncio.run(enricher.extract_batch(source))
