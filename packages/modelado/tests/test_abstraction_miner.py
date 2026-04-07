from __future__ import annotations

from modelado.abstraction_miner import (
    AbstractionPolicy,
    GraphEdge,
    mine_abstractions,
    summarize_motifs,
)


def _edges_variant(order: int) -> list[GraphEdge]:
    edges = [
        GraphEdge(out_id="a", in_id="b", edge_label="knowledge:supports"),
        GraphEdge(out_id="b", in_id="c", edge_label="knowledge:supports"),
        GraphEdge(out_id="a", in_id="x", edge_label="knowledge:supports"),
        GraphEdge(out_id="x", in_id="c", edge_label="knowledge:supports"),
    ]
    if order == 1:
        return list(reversed(edges))
    return edges


def test_abstraction_miner_deterministic_across_edge_order() -> None:
    policy = AbstractionPolicy(space_id="space-1", min_score=1.0)
    result_a = mine_abstractions(artifact_id="art-1", edges=_edges_variant(0), policy=policy)
    result_b = mine_abstractions(artifact_id="art-1", edges=_edges_variant(1), policy=policy)

    assert summarize_motifs(result_a) == summarize_motifs(result_b)
    assert [e.idempotency_key for e in result_a.edge_events] == [
        e.idempotency_key for e in result_b.edge_events
    ]


def test_abstraction_miner_respects_min_score() -> None:
    policy = AbstractionPolicy(space_id="space-1", min_score=10.0)
    result = mine_abstractions(artifact_id="art-1", edges=_edges_variant(0), policy=policy)
    assert result.abstraction_fragments == []
    assert result.edge_events == []


def test_abstraction_miner_records_evidence() -> None:
    policy = AbstractionPolicy(space_id="space-1", min_score=1.0, benefit=2.0, cost=0.0)
    result = mine_abstractions(artifact_id="art-1", edges=_edges_variant(0), policy=policy)
    assert result.abstraction_fragments
    frag = result.abstraction_fragments[0]
    relation_payload = getattr(frag, "value", None)
    if relation_payload is None:
        relation_payload = getattr(frag, "content", None)
    assert relation_payload is not None

    qualifiers = relation_payload.qualifiers
    provenance = qualifiers.get("provenance", {})
    assert qualifiers["expands_to"]
    assert qualifiers["reason_codes"] == ["structural_path_compress"]
    assert provenance["scoring_policy"]["benefit"] == 2.0
