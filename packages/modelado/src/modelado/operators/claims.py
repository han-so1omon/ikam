from __future__ import annotations

import json
from typing import Any

from blake3 import blake3

from .core import Operator, OperatorEnv, OperatorParams, ProvenanceRecord, record_provenance


CLAIM_MIME = "application/vnd.ikam.claim+json"


def _claim_fragment_ref(value: dict[str, Any]) -> str:
    stable_json = json.dumps(
        {"mime_type": CLAIM_MIME, "value": value},
        sort_keys=True,
        ensure_ascii=False,
    )
    return blake3(stable_json.encode("utf-8")).hexdigest()


class ClaimsOperator(Operator):
    def apply(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> dict[str, Any]:
        del fragment, env
        raw = params.parameters
        entity_relationship_set = raw.get("entity_relationship_set") if isinstance(raw.get("entity_relationship_set"), dict) else {}
        source_subgraph_ref = str(entity_relationship_set.get("source_subgraph_ref") or entity_relationship_set.get("subgraph_ref") or "")
        default_subgraph_ref = f"{source_subgraph_ref}:claims" if source_subgraph_ref else ""
        subgraph_ref = str(raw.get("subgraph_ref") or default_subgraph_ref)
        entity_relationships = (
            entity_relationship_set.get("entity_relationships") if isinstance(entity_relationship_set.get("entity_relationships"), list) else []
        )

        claim_rows: list[dict[str, Any]] = []
        fragment_ids: list[str] = []
        fragment_artifact_map: dict[str, str] = {}

        for item in entity_relationships:
            if not isinstance(item, dict):
                continue
            value = item.get("value") if isinstance(item.get("value"), dict) else {}
            relationships = value.get("relationships") if isinstance(value.get("relationships"), list) else []
            for relationship in relationships:
                if not isinstance(relationship, dict):
                    continue
                subject = str(relationship.get("source") or "")
                predicate = str(relationship.get("relationship") or "")
                object_ = str(relationship.get("target") or "")
                if not (subject and predicate and object_):
                    continue
                row = {
                    "entity_relationship_fragment_id": str(item.get("cas_id") or value.get("fragment_id") or ""),
                    "chunk_fragment_id": str(value.get("chunk_fragment_id") or ""),
                    "chunk_id": str(value.get("chunk_id") or ""),
                    "document_id": str(value.get("document_id") or ""),
                    "artifact_id": str(value.get("artifact_id") or ""),
                    "filename": str(value.get("filename") or ""),
                    "span": value.get("span") if isinstance(value.get("span"), dict) else None,
                    "order": value.get("order"),
                    "source_text": str(value.get("text") or ""),
                    "subject": subject,
                    "predicate": predicate,
                    "object": object_,
                    "claim": f"{subject} {predicate} {object_}",
                }
                fragment_id = _claim_fragment_ref(row)
                row["fragment_id"] = fragment_id
                claim_rows.append(row)
                fragment_ids.append(fragment_id)
                artifact_id = row["artifact_id"]
                if artifact_id:
                    fragment_artifact_map[fragment_id] = artifact_id

        return {
            "status": "ok",
            "source_kind": "entity_relationship_set",
            "fragment_ids": fragment_ids,
            "fragment_artifact_map": fragment_artifact_map,
            "claims": claim_rows,
            "claim_set": {
                "kind": "claim_set",
                "source_subgraph_ref": source_subgraph_ref,
                "subgraph_ref": subgraph_ref,
                "claim_refs": fragment_ids,
                "claims": [
                    {
                        "cas_id": row["fragment_id"],
                        "mime_type": CLAIM_MIME,
                        "value": row,
                    }
                    for row in claim_rows
                ],
            },
            "summary": {
                "entity_relationship_count": len(
                    [item for item in entity_relationships if isinstance(item, dict)]
                ),
                "claim_fragment_count": len(claim_rows),
                "claim_count": len(claim_rows),
            },
        }

    def provenance(self, params: OperatorParams, env: OperatorEnv) -> ProvenanceRecord:
        return record_provenance(params, env)
