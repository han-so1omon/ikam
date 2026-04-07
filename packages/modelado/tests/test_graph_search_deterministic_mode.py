"""Graph search deterministic mode tests — §8.1 query repeatability.

Contract: Identical graph + query → identical ranking in deterministic mode.
Tie-breaking policy (by fragment_id) must be consistent.
Stochastic mode varies but deterministic mode does not.

Pure in-memory tests — no database required.

References:
    - IKAM_MONOID_ALGEBRA_CONTRACT.md §8.1
    - packages/modelado/src/modelado/graph_search/scoring.py
    - packages/modelado/src/modelado/graph_search/fusion.py
"""

from __future__ import annotations

import pytest

from modelado.graph_search.fusion import fuse_candidates
from modelado.graph_search.scoring import score_candidates


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_fused(*entries: tuple[str, list[str]]) -> dict:
    """Build fused candidate dict from (fragment_id, evidence) pairs."""
    content = [{"fragment_id": fid, "evidence": ev} for fid, ev in entries]
    return fuse_candidates(content=content, relations=[], edges=[])


def _ranking_ids(ranked: list[dict]) -> list[str]:
    """Extract fragment_id ordering from ranked results."""
    return [r["fragment_id"] for r in ranked]


def _ranking_scores(ranked: list[dict]) -> list[float]:
    """Extract score values from ranked results."""
    return [r["score"] for r in ranked]


# ---------------------------------------------------------------------------
# §8.1 — Deterministic mode produces stable rankings
# ---------------------------------------------------------------------------


class TestDeterministicModeStability:
    """Identical inputs → identical rankings in deterministic mode."""

    def test_identical_inputs_identical_rankings(self):
        """Core contract: same fused + query → same ranking, 10 runs."""
        fused = _make_fused(
            ("f1", ["content-match", "title-match"]),
            ("f2", ["content-match"]),
            ("f3", ["content-match", "relation-hit", "graph-edge"]),
        )
        query = "what is the revenue model?"

        baseline = score_candidates(fused, query, mode="deterministic")
        baseline_ids = _ranking_ids(baseline)
        baseline_scores = _ranking_scores(baseline)

        for _ in range(10):
            result = score_candidates(fused, query, mode="deterministic")
            assert _ranking_ids(result) == baseline_ids, "Ranking order must be stable"
            assert _ranking_scores(result) == baseline_scores, "Scores must be identical"

    def test_different_queries_may_produce_different_rankings(self):
        """Different queries may produce different scores (but each is internally stable)."""
        fused = _make_fused(
            ("f1", ["alpha"]),
            ("f2", ["beta", "gamma"]),
        )

        r_q1 = score_candidates(fused, "query-one", mode="deterministic")
        r_q2 = score_candidates(fused, "query-two", mode="deterministic")

        # Each query is internally deterministic
        assert _ranking_ids(r_q1) == _ranking_ids(
            score_candidates(fused, "query-one", mode="deterministic")
        )
        assert _ranking_ids(r_q2) == _ranking_ids(
            score_candidates(fused, "query-two", mode="deterministic")
        )

    def test_single_candidate_deterministic(self):
        """Edge case: single candidate always returns that candidate."""
        fused = _make_fused(("f-only", ["match"]))
        r = score_candidates(fused, "q", mode="deterministic")
        assert len(r) == 1
        assert r[0]["fragment_id"] == "f-only"

    def test_empty_candidates_returns_empty(self):
        """Edge case: no candidates → empty result."""
        fused = fuse_candidates(content=[], relations=[], edges=[])
        r = score_candidates(fused, "q", mode="deterministic")
        assert r == []


# ---------------------------------------------------------------------------
# §8.1 — Tie-breaking by fragment_id
# ---------------------------------------------------------------------------


class TestTieBreakingPolicy:
    """When scores are equal, tie-break by fragment_id (lexicographic ascending)."""

    def test_tied_scores_broken_by_fragment_id(self):
        """Fragments with identical evidence → identical scores → sort by fragment_id."""
        fused = _make_fused(
            ("f-zebra", ["match"]),
            ("f-alpha", ["match"]),
            ("f-middle", ["match"]),
        )
        ranked = score_candidates(fused, "q", mode="deterministic")

        # All have same evidence → same scores → sort by fragment_id ascending
        ids = _ranking_ids(ranked)
        assert ids == sorted(ids), (
            f"Tie-break must sort by fragment_id ascending: {ids}"
        )

    def test_tie_break_stability_across_runs(self):
        """Tie-broken ordering must be stable across repeated runs."""
        fused = _make_fused(
            ("b", ["x"]),
            ("a", ["x"]),
            ("c", ["x"]),
        )

        baseline_ids = _ranking_ids(score_candidates(fused, "q", mode="deterministic"))
        for _ in range(20):
            assert _ranking_ids(score_candidates(fused, "q", mode="deterministic")) == baseline_ids

    def test_non_tied_scores_order_by_score_descending(self):
        """When scores differ, higher score ranks first regardless of fragment_id."""
        fused = _make_fused(
            ("zzz-low", ["a"]),                           # 1 evidence item
            ("aaa-high", ["a", "b", "c", "d", "e"]),      # 5 evidence items → higher score
        )
        ranked = score_candidates(fused, "q", mode="deterministic")
        assert ranked[0]["fragment_id"] == "aaa-high", (
            "Higher-scoring fragment should rank first"
        )
        assert ranked[0]["score"] > ranked[1]["score"]


# ---------------------------------------------------------------------------
# §8.1 — Stochastic mode varies, deterministic does not
# ---------------------------------------------------------------------------


class TestStochasticVsDeterministic:
    """Stochastic mode introduces perturbation; deterministic does not."""

    def test_deterministic_scores_are_exact(self):
        """Deterministic mode: scores must be float-identical across runs."""
        fused = _make_fused(("f1", ["a", "b"]), ("f2", ["c"]))
        baseline = score_candidates(fused, "q", mode="deterministic")
        for _ in range(10):
            result = score_candidates(fused, "q", mode="deterministic")
            for b, r in zip(baseline, result):
                assert b["score"] == r["score"], (
                    f"Deterministic scores must be exact: {b['score']} != {r['score']}"
                )

    def test_stochastic_mode_perturbs_scores(self):
        """Stochastic mode adds perturbation within ±0.05 range."""
        fused = _make_fused(("f1", ["a"]), ("f2", ["b"]))
        det = score_candidates(fused, "q", mode="deterministic")
        sto = score_candidates(fused, "q", mode="stochastic")

        # At least one score should differ (perturbation applied)
        det_scores = {r["fragment_id"]: r["score"] for r in det}
        sto_scores = {r["fragment_id"]: r["score"] for r in sto}

        diffs = [abs(det_scores[fid] - sto_scores[fid]) for fid in det_scores]
        # Perturbation is seeded per (fid, query), so it's deterministic per call,
        # but differs from the un-perturbed deterministic score.
        # At least one fragment should have a different score.
        assert any(d > 0 for d in diffs), (
            "Stochastic mode should perturb at least one score"
        )

        # All perturbations should be within ±0.05
        for fid in det_scores:
            assert abs(det_scores[fid] - sto_scores[fid]) <= 0.05 + 1e-9, (
                f"Stochastic perturbation for {fid} exceeds ±0.05 range"
            )

    def test_stochastic_mode_is_seeded_deterministically(self):
        """Stochastic mode uses seeded RNG, so same inputs → same perturbed output."""
        fused = _make_fused(("f1", ["x"]), ("f2", ["y", "z"]))
        r1 = score_candidates(fused, "q", mode="stochastic")
        r2 = score_candidates(fused, "q", mode="stochastic")

        # Stochastic mode is seeded per (fid, query) — repeated calls with
        # same inputs must produce same perturbed scores.
        assert _ranking_scores(r1) == _ranking_scores(r2), (
            "Stochastic mode with same seed must be deterministic"
        )


# ---------------------------------------------------------------------------
# Trace metadata correctness
# ---------------------------------------------------------------------------


class TestTraceMetadata:
    """Score results include trace with mode, signals, and weights."""

    def test_trace_contains_mode(self):
        """Trace must reflect the scoring mode used."""
        fused = _make_fused(("f1", ["a"]))
        for mode in ("deterministic", "stochastic"):
            ranked = score_candidates(fused, "q", mode=mode)
            assert ranked[0]["trace"]["mode"] == mode

    def test_trace_signals_present(self):
        """Trace must include text, semantic, and graph signal scores."""
        fused = _make_fused(("f1", ["a", "b"]))
        ranked = score_candidates(fused, "q", mode="deterministic")
        signals = ranked[0]["trace"]["signals"]
        assert "text" in signals
        assert "semantic" in signals
        assert "graph" in signals

    def test_trace_weights_sum_to_one(self):
        """Fusion weights must be normalized (sum to 1.0)."""
        fused = _make_fused(("f1", ["a"]))
        ranked = score_candidates(fused, "q", mode="deterministic")
        weights = ranked[0]["trace"]["weights"]
        total = sum(weights.values())
        assert abs(total - 1.0) < 1e-6, f"Weights must sum to 1.0, got {total}"
