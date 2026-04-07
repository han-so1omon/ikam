from __future__ import annotations

import hashlib
import json
import re
from dataclasses import dataclass


def _sha256(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def _canonical_text(value: str) -> str:
    collapsed = re.sub(r"\s+", " ", (value or "").strip().lower())
    return collapsed


def stable_entity_key(source_fragment_id: str, entity_label: str) -> str:
    canonical = _canonical_text(entity_label)
    return _sha256(f"entity|{source_fragment_id}|{canonical}")


def stable_relation_key(
    source_fragment_id: str,
    predicate: str,
    source_label: str,
    target_label: str,
) -> str:
    canonical_predicate = _canonical_text(predicate)
    canonical_source = _canonical_text(source_label)
    canonical_target = _canonical_text(target_label)
    return _sha256(
        f"relation|{source_fragment_id}|{canonical_predicate}|{canonical_source}|{canonical_target}"
    )


def input_fingerprint(payload: object) -> str:
    encoded = json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return _sha256(encoded)


@dataclass(frozen=True)
class ExtractedEntity:
    label: str
    canonical_label: str
    source_fragment_id: str
    entity_key: str


@dataclass(frozen=True)
class ExtractedRelation:
    predicate: str
    source_label: str
    target_label: str
    source_entity_key: str
    target_entity_key: str
    relation_key: str


@dataclass(frozen=True)
class ExtractionBatch:
    input_fingerprint: str
    extractor_fingerprint: str
    policy_version: str
    mode: str
    entities: tuple[ExtractedEntity, ...]
    relations: tuple[ExtractedRelation, ...]
