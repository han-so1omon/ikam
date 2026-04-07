import asyncio
import json

from ikam.forja.contracts import stable_entity_key, stable_relation_key
from ikam.forja.enricher import EntityRelationEnricher
from ikam.fragments import Fragment


class _Result:
    def __init__(self, output: str):
        self.output = output


class _ClientA:
    async def call_model(self, prompt: str, model: str, temperature: float) -> _Result:
        return _Result(
            json.dumps(
                {
                    "entities": ["Acme", "Contoso"],
                    "relations": [{"source": "Acme", "target": "Contoso", "predicate": "Partners With"}],
                }
            )
        )


class _ClientB:
    async def call_model(self, prompt: str, model: str, temperature: float) -> _Result:
        return _Result(
            json.dumps(
                {
                    "entities": ["Contoso", "Acme", "AcmE"],
                    "relations": [
                        {"source": "acme", "target": "contoso", "predicate": "partners   with"},
                        {"source": "Acme", "target": "Contoso", "predicate": "semantic_link"},
                    ],
                }
            )
        )


def test_extraction_batch_contains_required_metadata() -> None:
    source = Fragment(value={"text": "Acme partners with Contoso."}, mime_type="text/plain")
    batch = asyncio.run(EntityRelationEnricher(ai_client=_ClientA()).extract_batch(source, mode="explore-fast"))

    assert batch.input_fingerprint
    assert batch.extractor_fingerprint
    assert batch.policy_version
    assert batch.mode == "explore-fast"
    assert batch.entities


def test_stable_entity_and_relation_keys_for_canonical_content() -> None:
    source_id = "fid-1"
    assert stable_entity_key(source_id, "Acme") == stable_entity_key(source_id, " acme  ")
    assert stable_relation_key(source_id, "Partners With", "Acme", "Contoso") == stable_relation_key(
        source_id,
        "partners   with",
        "acme",
        " contoso ",
    )


def test_exploration_mode_candidate_variability_keeps_contract_valid() -> None:
    source = Fragment(value={"text": "Acme partners with Contoso."}, mime_type="text/plain")
    batch_a = asyncio.run(EntityRelationEnricher(ai_client=_ClientA()).extract_batch(source, mode="explore-graph"))
    batch_b = asyncio.run(EntityRelationEnricher(ai_client=_ClientB()).extract_batch(source, mode="explore-graph"))

    assert batch_a.mode == "explore-graph"
    assert batch_b.mode == "explore-graph"
    assert batch_a.input_fingerprint and batch_b.input_fingerprint
    assert batch_a.extractor_fingerprint == batch_b.extractor_fingerprint
    assert len(batch_b.entities) >= len(batch_a.entities)
