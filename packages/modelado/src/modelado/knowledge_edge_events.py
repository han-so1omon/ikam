"""Deterministic mapping from IKAM relation fragments → graph edge events.

This is the O6 → O4 bridge:
- O6: relation fragments (first-class knowledge links)
- O4: authoritative append-only Postgres edge-event log (graph_edge_events)

The mapping must be:
- deterministic (stable across processes)
- idempotent (safe to retry)
- cycle-tolerant (no acyclicity enforcement)

This module is pure logic (no DB/Kafka).
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from typing import Any, Iterable, Mapping

from modelado.graph_edge_event_log import compute_edge_event_idempotency_key, compute_edge_identity_key


_PREDICATE_RX = re.compile(r"[^a-z0-9_]+")


def canonicalize_predicate(predicate: str) -> str:
    """Canonicalize predicates for stable edge labels."""

    cleaned = (predicate or "").strip().lower()
    cleaned = re.sub(r"\s+", "_", cleaned)
    cleaned = _PREDICATE_RX.sub("_", cleaned)
    cleaned = re.sub(r"_+", "_", cleaned).strip("_")
    return cleaned or "relation"


def _unique_sorted(ids: Iterable[str]) -> list[str]:
    out: list[str] = []
    seen: set[str] = set()
    for raw in ids:
        if not isinstance(raw, str):
            continue
        val = raw.strip()
        if not val or val in seen:
            continue
        seen.add(val)
        out.append(val)
    out.sort()
    return out


def iter_relation_pairs(
    *,
    subject_ids: Iterable[str],
    object_ids: Iterable[str],
    directed: bool,
) -> list[tuple[str, str]]:
    """Return a deterministic list of (out_id, in_id) pairs."""

    subjects = _unique_sorted(subject_ids)
    objects = _unique_sorted(object_ids)

    pairs: list[tuple[str, str]] = []

    for s in subjects:
        for o in objects:
            if s == o:
                continue
            if directed:
                pairs.append((s, o))
            else:
                a, b = (s, o) if s < o else (o, s)
                pairs.append((a, b))
                pairs.append((b, a))

    # Ensure stable ordering even if inputs had duplicates.
    pairs = sorted(set(pairs))
    return pairs


@dataclass(frozen=True)
class KnowledgeEdgeEventInput:
    op: str
    edge_label: str
    out_id: str
    in_id: str
    properties: dict[str, Any]
    idempotency_key: str
    edge_identity_key: str


def build_knowledge_edge_events(
    *,
    op: str,
    relation_fragment_id: str,
    predicate: str,
    subject_fragment_ids: Iterable[str],
    object_fragment_ids: Iterable[str],
    directed: bool,
    confidence_score: float,
    qualifiers: Mapping[str, Any] | None = None,
) -> list[KnowledgeEdgeEventInput]:
    """Build deterministic edge-event inputs for a relation fragment."""

    edge_label = f"knowledge:{canonicalize_predicate(predicate)}"

    base_properties: dict[str, Any] = {
        "relationFragmentId": str(relation_fragment_id),
        "predicate": str(predicate),
        "directed": bool(directed),
        "confidenceScore": float(confidence_score),
        "qualifiers": dict(qualifiers or {}),
    }

    events: list[KnowledgeEdgeEventInput] = []
    for out_id, in_id in iter_relation_pairs(
        subject_ids=subject_fragment_ids,
        object_ids=object_fragment_ids,
        directed=directed,
    ):
        key = compute_edge_event_idempotency_key(
            op=op,
            edge_label=edge_label,
            out_id=out_id,
            in_id=in_id,
            properties=base_properties,
        )
        identity = compute_edge_identity_key(
            edge_label=edge_label,
            out_id=out_id,
            in_id=in_id,
            properties=base_properties,
        )
        events.append(
            KnowledgeEdgeEventInput(
                op=op,
                edge_label=edge_label,
                out_id=out_id,
                in_id=in_id,
                properties=base_properties,
                idempotency_key=key,
                edge_identity_key=identity,
            )
        )

    return events
