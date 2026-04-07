from __future__ import annotations

import json
from typing import List, Protocol

from ikam.adapters import v3_fragment_to_cas_bytes
from ikam.fragments import BindingGroup, Fragment, RELATION_MIME, Relation, SlotBinding
from ikam.graph import _cas_hex
from ikam.forja.contracts import (
    ExtractedEntity,
    ExtractedRelation,
    ExtractionBatch,
    input_fingerprint,
    stable_entity_key,
    stable_relation_key,
)

ENTITY_MIME = "application/ikam-entity+json"


class LLMResult(Protocol):
    output: str


class AIClient(Protocol):
    async def call_model(self, prompt: str, model: str, temperature: float) -> LLMResult: ...


class EnrichmentError(Exception):
    """Raised when enrichment fails (LLM error, malformed response)."""


def _cas_fragment(value: object, mime_type: str) -> Fragment:
    fragment = Fragment(value=value, mime_type=mime_type)
    cas_id = _cas_hex(v3_fragment_to_cas_bytes(fragment))
    return Fragment(cas_id=cas_id, value=value, mime_type=mime_type)


def _extract_text(fragment: Fragment) -> str:
    value = fragment.value
    if isinstance(value, dict):
        if isinstance(value.get("text"), str):
            return value["text"]
        if isinstance(value.get("concept"), str):
            return value["concept"]
    if isinstance(value, str):
        return value
    return ""


def _extract_relations(raw: object) -> List[dict[str, str]]:
    if not isinstance(raw, list):
        return []
    relations: List[dict[str, str]] = []
    for item in raw:
        if not isinstance(item, dict):
            continue
        source = item.get("source")
        target = item.get("target")
        predicate = item.get("predicate")
        if not (isinstance(source, str) and isinstance(target, str) and isinstance(predicate, str)):
            continue
        relations.append({"source": source.strip(), "target": target.strip(), "predicate": predicate.strip()})
    return relations


async def _extract_with_llm(text: str, ai_client: AIClient) -> tuple[List[str], List[dict[str, str]]]:
    prompt = (
        "Extract entities and relations from this text. "
        "Return strict JSON object with keys entities (string list) and relations "
        "(list of {source,target,predicate} objects). Text: "
        f"{text}"
    )
    result = await ai_client.call_model(prompt=prompt, model="gpt-4o-mini", temperature=0.0)
    cleaned = (result.output or "").strip()
    if cleaned.startswith("```json"):
        cleaned = cleaned[7:]
    if cleaned.endswith("```"):
        cleaned = cleaned[:-3]
    payload = json.loads(cleaned.strip())
    if not isinstance(payload, dict):
        return [], []

    entities_raw = payload.get("entities")
    entities = [item.strip() for item in entities_raw if isinstance(item, str) and item.strip()] if isinstance(entities_raw, list) else []
    relations = _extract_relations(payload.get("relations"))
    return entities, relations


class EntityRelationEnricher:
    def __init__(self, ai_client: AIClient, *, extractor_fingerprint: str = "forja.entity-relation:v1"):
        self.ai_client = ai_client
        self.extractor_fingerprint = extractor_fingerprint

    async def extract_batch(
        self,
        fragment: Fragment,
        *,
        mode: str = "explore-fast",
        policy_version: str = "2026-02-10",
    ) -> ExtractionBatch:
        source_id = fragment.cas_id or _cas_hex(v3_fragment_to_cas_bytes(fragment))
        text = _extract_text(fragment)

        try:
            entities, relation_specs = await _extract_with_llm(text, self.ai_client)
        except json.JSONDecodeError as exc:
            raise EnrichmentError(f"LLM returned malformed JSON: {exc}") from exc
        except Exception as exc:
            raise EnrichmentError(f"LLM extraction failed: {exc}") from exc

        entity_models: list[ExtractedEntity] = []
        entity_by_label: dict[str, ExtractedEntity] = {}
        for label in entities:
            canonical = " ".join(label.strip().lower().split())
            if not canonical:
                continue
            model = ExtractedEntity(
                label=label.strip(),
                canonical_label=canonical,
                source_fragment_id=source_id,
                entity_key=stable_entity_key(source_id, label),
            )
            entity_models.append(model)
            entity_by_label[canonical] = model

        if not relation_specs:
            relation_specs = [
                {"source": text[:16] or "source", "target": entity.label, "predicate": "semantic_link"}
                for entity in entity_models
            ]

        relation_models: list[ExtractedRelation] = []
        for rel in relation_specs:
            source_label = str(rel.get("source") or "").strip()
            target_label = str(rel.get("target") or "").strip()
            predicate = str(rel.get("predicate") or "semantic_link").strip() or "semantic_link"
            if not source_label or not target_label:
                continue
            source_lookup = " ".join(source_label.lower().split())
            target_lookup = " ".join(target_label.lower().split())
            source_entity = entity_by_label.get(source_lookup)
            target_entity = entity_by_label.get(target_lookup)
            source_key = source_entity.entity_key if source_entity else stable_entity_key(source_id, source_label)
            target_key = target_entity.entity_key if target_entity else stable_entity_key(source_id, target_label)
            relation_models.append(
                ExtractedRelation(
                    predicate=predicate,
                    source_label=source_label,
                    target_label=target_label,
                    source_entity_key=source_key,
                    target_entity_key=target_key,
                    relation_key=stable_relation_key(source_id, predicate, source_label, target_label),
                )
            )

        return ExtractionBatch(
            input_fingerprint=input_fingerprint(
                {
                    "source_id": source_id,
                    "text": text,
                    "entity_count": len(entity_models),
                    "relation_count": len(relation_models),
                }
            ),
            extractor_fingerprint=self.extractor_fingerprint,
            policy_version=policy_version,
            mode=mode,
            entities=tuple(entity_models),
            relations=tuple(relation_models),
        )

    async def enrich(self, fragment: Fragment) -> List[Fragment]:
        """Extract entity + semantic-link relation fragments from a source fragment."""
        batch = await self.extract_batch(fragment)
        if not batch.entities:
            return []

        source_id = fragment.cas_id or _cas_hex(v3_fragment_to_cas_bytes(fragment))
        entity_fragments: List[Fragment] = []
        relation_fragments: List[Fragment] = []
        extractor = "llm"

        entity_ids_by_key: dict[str, str] = {}
        for entity in batch.entities:
            entity_fragment = _cas_fragment(
                {
                    "entity": entity.label,
                    "entity_key": entity.entity_key,
                    "source_fragment_id": source_id,
                    "extractor": extractor,
                },
                ENTITY_MIME,
            )
            entity_fragments.append(entity_fragment)
            entity_ids_by_key[entity.entity_key] = entity_fragment.cas_id or source_id

        for rel in batch.relations:
            source_entity_id = entity_ids_by_key.get(rel.source_entity_key, source_id)
            target_entity_id = entity_ids_by_key.get(rel.target_entity_key, source_id)
            relation = Relation(
                predicate=rel.predicate or "semantic_link",
                directed=True,
                confidence_score=0.7,
                qualifiers={
                    "extractor": extractor,
                    "relation_key": rel.relation_key,
                    "source_entity_key": rel.source_entity_key,
                    "target_entity_key": rel.target_entity_key,
                },
                binding_groups=[
                    BindingGroup(
                        invocation_id=f"enrich:{source_id}:{source_entity_id}:{target_entity_id}",
                        slots=[
                            SlotBinding(slot="source", fragment_id=source_entity_id),
                            SlotBinding(slot="entity", fragment_id=target_entity_id),
                        ],
                    )
                ],
            )
            relation_fragments.append(_cas_fragment(relation.model_dump(mode="json"), RELATION_MIME))

        return [*entity_fragments, *relation_fragments]
