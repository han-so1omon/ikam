import asyncio
import json

from ikam.forja.enricher import ENTITY_MIME, EntityRelationEnricher
from ikam.fragments import Fragment, RELATION_MIME


class _MockLLMResult:
    def __init__(self, output: str):
        self.output = output


class _MockAIClient:
    async def call_model(self, prompt: str, model: str, temperature: float) -> _MockLLMResult:
        return _MockLLMResult(
            json.dumps(
                {
                    "entities": ["Acme", "Contoso"],
                    "relations": [
                        {"source": "Acme", "target": "Contoso", "predicate": "partners_with"}
                    ],
                }
            )
        )


def test_enricher_emits_entity_and_relation_fragments():
    source = Fragment(value={"text": "Acme partners with Contoso in Mexico City."}, mime_type="text/plain")

    result = asyncio.run(EntityRelationEnricher(ai_client=_MockAIClient()).enrich(source))

    entity_frags = [f for f in result if f.mime_type == ENTITY_MIME]
    relation_frags = [f for f in result if f.mime_type == RELATION_MIME]

    assert entity_frags
    assert relation_frags
    assert all("entity" in (f.value or {}) for f in entity_frags)
    relation_payload = relation_frags[0].value or {}
    assert relation_payload["predicate"] == "partners_with"


def test_enricher_uses_llm_extractor_when_available():
    source = Fragment(value={"text": "Acme partners with Contoso."}, mime_type="text/plain")

    result = asyncio.run(EntityRelationEnricher(ai_client=_MockAIClient()).enrich(source))

    entity_frags = [f for f in result if f.mime_type == ENTITY_MIME]
    relation_frags = [f for f in result if f.mime_type == RELATION_MIME]

    assert entity_frags
    assert relation_frags
    assert all((f.value or {}).get("extractor") == "llm" for f in entity_frags)
    assert any((f.value or {}).get("predicate") == "partners_with" for f in relation_frags)
