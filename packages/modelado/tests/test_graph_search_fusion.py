from modelado.graph_search.fusion import fuse_candidates
from modelado.graph_search.scoring import score_candidates


def test_fuse_candidates_merges_evidence():
    fused = fuse_candidates(
        content=[{"fragment_id": "f1", "evidence": ["content"]}],
        relations=[{"fragment_id": "f1", "evidence": ["relation"]}],
        edges=[{"fragment_id": "f2", "evidence": ["edge"]}],
    )
    assert fused["f1"]["evidence"] == ["content", "relation"]
    assert "f2" in fused


def test_parser_trace_records_signal_contributions_and_mode():
    """Contract: C3, §7, §13.

    Score results must include a trace with mode and per-signal contributions.
    The pipeline is generic — no intent-specific branching.
    """
    fused = fuse_candidates(
        content=[{"fragment_id": "f1", "evidence": ["a"]}],
        relations=[{"fragment_id": "f1", "evidence": ["r"]}],
        edges=[{"fragment_id": "f1", "evidence": ["e"]}],
    )
    ranked = score_candidates(fused, query="what kind of business is this?", mode="stochastic")
    assert ranked
    assert ranked[0]["trace"]["mode"] == "stochastic"
    assert set(ranked[0]["trace"]["signals"].keys()) >= {"text", "semantic", "graph"}


def test_deterministic_mode_produces_stable_ordering():
    """Contract: §8.1 — identical inputs produce identical rankings."""
    fused = fuse_candidates(
        content=[
            {"fragment_id": "f1", "evidence": ["a"]},
            {"fragment_id": "f2", "evidence": ["b", "c"]},
        ],
        relations=[],
        edges=[],
    )
    r1 = score_candidates(fused, query="test", mode="deterministic")
    r2 = score_candidates(fused, query="test", mode="deterministic")
    assert [r["fragment_id"] for r in r1] == [r["fragment_id"] for r in r2]
    assert [r["score"] for r in r1] == [r["score"] for r in r2]


def test_score_candidates_returns_normalized_weights():
    """Contract: §7.2 — results include normalized fusion weights."""
    fused = fuse_candidates(
        content=[{"fragment_id": "f1", "evidence": ["x"]}],
        relations=[],
        edges=[],
    )
    ranked = score_candidates(fused, query="q", mode="deterministic")
    assert ranked[0]["trace"]["weights"]
    total = sum(ranked[0]["trace"]["weights"].values())
    assert abs(total - 1.0) < 1e-6, f"Weights must sum to 1.0, got {total}"
