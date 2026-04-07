from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Iterable, Mapping, Sequence
import hashlib
import json

from ikam.fragments import Fragment as V3Fragment, Relation, RELATION_MIME
from ikam.graph import _cas_hex

from modelado.graph_edge_event_log import compute_edge_identity_key
from modelado.knowledge_edge_events import KnowledgeEdgeEventInput, build_knowledge_edge_events


def _stable_json(payload: Mapping[str, Any]) -> str:
    return json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def _hash_hex(payload: Mapping[str, Any]) -> str:
    blob = _stable_json(payload).encode("utf-8")
    return hashlib.blake2b(blob, digest_size=16).hexdigest()


def _unique_sorted(values: Iterable[str]) -> list[str]:
    out: list[str] = []
    seen: set[str] = set()
    for raw in values:
        if not isinstance(raw, str):
            continue
        val = raw.strip()
        if not val or val in seen:
            continue
        seen.add(val)
        out.append(val)
    out.sort()
    return out


@dataclass(frozen=True)
class GraphEdge:
    out_id: str
    in_id: str
    edge_label: str
    properties: dict[str, Any] = field(default_factory=dict)


@dataclass(frozen=True)
class MotifOccurrence:
    out_id: str
    mid_id: str
    in_id: str
    edge1_key: str
    edge2_key: str


@dataclass(frozen=True)
class MotifStats:
    motif_key: str
    count: int
    score: float
    occurrences: list[MotifOccurrence]


@dataclass(frozen=True)
class AbstractionPolicy:
    space_id: str
    base_tier: int = 0
    benefit: float = 1.0
    cost: float = 0.0
    min_score: float = 1.0
    predicate: str = "abstracts_path"
    reasons_dict_version: str = "v1"
    reason_codes: tuple[str, ...] = ("structural_path_compress",)


@dataclass(frozen=True)
class AbstractionResult:
    policy: AbstractionPolicy
    motifs: list[MotifStats]
    abstraction_fragments: list[V3Fragment]
    edge_events: list[KnowledgeEdgeEventInput]


def _edge_key(edge: GraphEdge) -> str:
    return compute_edge_identity_key(
        edge_label=edge.edge_label,
        out_id=edge.out_id,
        in_id=edge.in_id,
        properties=edge.properties,
    )


def _build_motif_key(policy: AbstractionPolicy, edge1: GraphEdge, edge2: GraphEdge) -> str:
    payload = {
        "space_id": policy.space_id,
        "tier": policy.base_tier,
        "edge1": edge1.edge_label,
        "edge2": edge2.edge_label,
    }
    return _stable_json(payload)


def _score_motif(policy: AbstractionPolicy, count: int) -> float:
    return float(count) * float(policy.benefit) - float(policy.cost)


def mine_abstractions(
    *,
    artifact_id: str,
    edges: Sequence[GraphEdge],
    policy: AbstractionPolicy,
) -> AbstractionResult:
    """Deterministic O7 abstraction miner over 2-path motifs."""
    if not edges:
        raise ValueError("O7 requires at least one edge")

    outgoing: dict[str, list[GraphEdge]] = {}
    incoming: dict[str, list[GraphEdge]] = {}
    for edge in edges:
        outgoing.setdefault(edge.out_id, []).append(edge)
        incoming.setdefault(edge.in_id, []).append(edge)

    motifs: dict[str, list[MotifOccurrence]] = {}

    for mid_id in sorted(set(outgoing.keys()) & set(incoming.keys())):
        in_edges = sorted(incoming.get(mid_id, []), key=_edge_key)
        out_edges = sorted(outgoing.get(mid_id, []), key=_edge_key)
        for e1 in in_edges:
            for e2 in out_edges:
                if e1.out_id == e2.in_id:
                    continue
                motif_key = _build_motif_key(policy, e1, e2)
                occurrence = MotifOccurrence(
                    out_id=e1.out_id,
                    mid_id=mid_id,
                    in_id=e2.in_id,
                    edge1_key=_edge_key(e1),
                    edge2_key=_edge_key(e2),
                )
                motifs.setdefault(motif_key, []).append(occurrence)

    motif_stats: list[MotifStats] = []
    for key, occurrences in motifs.items():
        count = len(occurrences)
        score = _score_motif(policy, count)
        motif_stats.append(
            MotifStats(
                motif_key=key,
                count=count,
                score=score,
                occurrences=sorted(occurrences, key=lambda o: (o.out_id, o.mid_id, o.in_id, o.edge1_key, o.edge2_key)),
            )
        )

    motif_stats.sort(key=lambda m: (-m.score, m.motif_key))
    chosen = motif_stats[0] if motif_stats else None

    abstraction_fragments: list[V3Fragment] = []
    edge_events: list[KnowledgeEdgeEventInput] = []

    if chosen and chosen.score >= policy.min_score:
        for occurrence in chosen.occurrences:
            qualifiers = {
                "space_id": policy.space_id,
                "tier": policy.base_tier + 1,
                "expands_to": [occurrence.edge1_key, occurrence.edge2_key],
                "motif_key": chosen.motif_key,
                "intermediate_id": occurrence.mid_id,
                "reasons_dict_version": policy.reasons_dict_version,
                "reason_codes": list(policy.reason_codes),
            }
            provenance = {
                "scoring_policy": {
                    "benefit": policy.benefit,
                    "cost": policy.cost,
                    "min_score": policy.min_score,
                },
                "motif_count": chosen.count,
                "evidence_edges": [occurrence.edge1_key, occurrence.edge2_key],
            }

            content = Relation(
                predicate=policy.predicate,
                directed=True,
                confidence_score=1.0,
                qualifiers={
                    **qualifiers,
                    "subject_fragment_ids": [occurrence.out_id],
                    "object_fragment_ids": [occurrence.in_id],
                    "provenance": provenance,
                },
            )
            payload = json.dumps(content.model_dump(mode="json"), sort_keys=True).encode("utf-8")
            relation_fragment_id = _cas_hex(payload)
            fragment = V3Fragment(
                cas_id=relation_fragment_id,
                value=content,
                mime_type=RELATION_MIME,
            )
            abstraction_fragments.append(fragment)

            edge_events.extend(
                build_knowledge_edge_events(
                    op="upsert",
                    relation_fragment_id=relation_fragment_id,
                    predicate=policy.predicate,
                    subject_fragment_ids=[occurrence.out_id],
                    object_fragment_ids=[occurrence.in_id],
                    directed=True,
                    confidence_score=1.0,
                    qualifiers=qualifiers,
                )
            )

    return AbstractionResult(
        policy=policy,
        motifs=motif_stats,
        abstraction_fragments=abstraction_fragments,
        edge_events=edge_events,
    )


def summarize_motifs(result: AbstractionResult) -> dict[str, Any]:
    """Return a deterministic summary payload for provenance."""
    return {
        "space_id": result.policy.space_id,
        "base_tier": result.policy.base_tier,
        "motifs": [
            {
                "motif_key": m.motif_key,
                "count": m.count,
                "score": m.score,
            }
            for m in result.motifs
        ],
    }
