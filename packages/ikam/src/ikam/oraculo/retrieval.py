"""Lightweight semantic retrieval utilities for GraphState traversal."""
from __future__ import annotations

import math
import re

from ikam.forja.contracts import ExtractedEntity, ExtractedRelation
from ikam.fragments import Fragment


def _tokenize(text: str) -> set[str]:
    return {token for token in re.findall(r"[a-z0-9]+", text.lower()) if len(token) > 1}


def _fragment_text(fragment: Fragment) -> str:
    return str(fragment.value or "")


def retrieve_relevant_fragments(
    *,
    query: str,
    fragments: list[Fragment],
    entities: list[ExtractedEntity],
    relations: list[ExtractedRelation],
    top_k: int = 8,
) -> tuple[list[Fragment], list[str]]:
    query_tokens = _tokenize(query)
    if not fragments:
        return [], ["no fragments available"]

    entity_names = [entity.label.lower() for entity in entities]
    relation_kinds = [relation.predicate.lower() for relation in relations]

    scored: list[tuple[float, Fragment, str]] = []
    for fragment in fragments:
        text = _fragment_text(fragment)
        text_tokens = _tokenize(text)
        if not text_tokens:
            continue

        overlap = len(query_tokens & text_tokens)
        lexical_score = overlap / max(1, len(query_tokens))

        text_lower = text.lower()
        entity_hits = sum(1 for name in entity_names if name and name in text_lower)
        relation_hits = sum(1 for kind in relation_kinds if kind and kind in text_lower)

        score = lexical_score + (0.12 * min(entity_hits, 3)) + (0.10 * min(relation_hits, 2))
        if score <= 0:
            continue

        reason = f"lexical={lexical_score:.2f}, entities={entity_hits}, relations={relation_hits}"
        scored.append((score, fragment, reason))

    if not scored:
        sample = fragments[: min(top_k, len(fragments))]
        return sample, ["no scored candidates; fallback to first fragments"]

    scored.sort(key=lambda item: item[0], reverse=True)
    k = min(top_k, max(3, int(math.sqrt(len(scored))) + 1))
    selected = scored[:k]
    selected_fragments = [fragment for _, fragment, _ in selected]
    reasoning = [f"top_k={k}"] + [f"{(fragment.cas_id or 'fragment')} score={score:.2f} ({why})" for score, fragment, why in selected]
    return selected_fragments, reasoning
