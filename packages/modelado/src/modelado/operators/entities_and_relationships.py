from __future__ import annotations

import json
import re
from typing import Any

from blake3 import blake3

from .core import Operator, OperatorEnv, OperatorParams, ProvenanceRecord, record_provenance


ENTITY_RELATIONSHIP_MIME = "application/vnd.ikam.entity-relationship+json"
_ENTITY_PATTERN = re.compile(r"\b[A-Z][a-z]+(?:\s+[A-Z][a-z]+)*\b")


def _entity_relationship_fragment_ref(value: dict[str, Any]) -> str:
    stable_json = json.dumps(
        {"mime_type": ENTITY_RELATIONSHIP_MIME, "value": value},
        sort_keys=True,
        ensure_ascii=False,
    )
    return blake3(stable_json.encode("utf-8")).hexdigest()


def _extract_entities(text: str) -> list[dict[str, str]]:
    seen: set[str] = set()
    entities: list[dict[str, str]] = []
    for match in _ENTITY_PATTERN.findall(text):
        if match in seen:
            continue
        seen.add(match)
        entities.append({"name": match, "type": "candidate_entity"})
    return entities


def _cooccurrence_relationships(entities: list[dict[str, str]]) -> list[dict[str, str]]:
    names = [entity["name"] for entity in entities]
    relationships: list[dict[str, str]] = []
    for left_index, source in enumerate(names):
        for target in names[left_index + 1 :]:
            relationships.append(
                {
                    "source": source,
                    "target": target,
                    "relationship": "co_occurs_in_chunk",
                }
            )
    return relationships


class EntitiesAndRelationshipsOperator(Operator):
    def apply(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> dict[str, Any]:
        del fragment, env
        raw = params.parameters
        chunk_extraction_set = raw.get("chunk_extraction_set") if isinstance(raw.get("chunk_extraction_set"), dict) else {}
        source_subgraph_ref = str(chunk_extraction_set.get("subgraph_ref") or "")
        subgraph_ref = str(raw.get("subgraph_ref") or _derived_subgraph_ref(source_subgraph_ref, suffix="entities"))
        chunk_extractions = (
            chunk_extraction_set.get("chunk_extractions") if isinstance(chunk_extraction_set.get("chunk_extractions"), list) else []
        )

        entity_relationship_rows: list[dict[str, Any]] = []
        fragment_ids: list[str] = []
        fragment_artifact_map: dict[str, str] = {}
        unique_entities: set[str] = set()
        relationship_count = 0

        for extraction in chunk_extractions:
            if not isinstance(extraction, dict):
                continue
            value = extraction.get("value") if isinstance(extraction.get("value"), dict) else {}
            text = str(value.get("text") or "")
            entities = _extract_entities(text)
            relationships = _cooccurrence_relationships(entities)
            row = {
                "chunk_fragment_id": str(extraction.get("cas_id") or ""),
                "chunk_id": str(value.get("chunk_id") or ""),
                "document_id": str(value.get("document_id") or ""),
                "artifact_id": str(value.get("artifact_id") or ""),
                "filename": str(value.get("filename") or ""),
                "span": value.get("span") if isinstance(value.get("span"), dict) else None,
                "order": value.get("order"),
                "text": text,
                "entities": entities,
                "relationships": relationships,
            }
            fragment_id = _entity_relationship_fragment_ref(row)
            row["fragment_id"] = fragment_id
            entity_relationship_rows.append(row)
            fragment_ids.append(fragment_id)
            artifact_id = row["artifact_id"]
            if artifact_id:
                fragment_artifact_map[fragment_id] = artifact_id
            unique_entities.update(entity["name"] for entity in entities)
            relationship_count += len(relationships)

        return {
            "status": "ok",
            "source_kind": "chunk_extraction_set",
            "fragment_ids": fragment_ids,
            "fragment_artifact_map": fragment_artifact_map,
            "entity_relationships": entity_relationship_rows,
            "entity_relationship_set": {
                "kind": "entity_relationship_set",
                "source_subgraph_ref": source_subgraph_ref,
                "subgraph_ref": subgraph_ref,
                "entity_relationship_refs": fragment_ids,
                "entity_relationships": [
                    {
                        "cas_id": row["fragment_id"],
                        "mime_type": ENTITY_RELATIONSHIP_MIME,
                        "value": row,
                    }
                    for row in entity_relationship_rows
                ],
            },
            "summary": {
                "chunk_count": len(entity_relationship_rows),
                "entity_relationship_fragment_count": len(entity_relationship_rows),
                "entity_count": len(unique_entities),
                "relationship_count": relationship_count,
            },
        }

    def provenance(self, params: OperatorParams, env: OperatorEnv) -> ProvenanceRecord:
        return record_provenance(params, env)


def _derived_subgraph_ref(source_subgraph_ref: str, *, suffix: str) -> str:
    if not source_subgraph_ref:
        return ""
    if source_subgraph_ref.startswith("subgraph://"):
        return f"{source_subgraph_ref}-{suffix}"
    return f"{source_subgraph_ref}:{suffix}"
