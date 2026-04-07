"""Mode-aware parser trace scoring. Contract: C3, sections 7, 8, 13.

Scores fused candidates with per-signal contributions and normalized weights.
One generic pipeline for all queries — no intent-specific branching.
"""

from __future__ import annotations

import hashlib
import random
from typing import Any

# Default signal weights (normalized to sum=1.0).
_DEFAULT_WEIGHTS = {"text": 0.4, "semantic": 0.35, "graph": 0.25}


def _text_signal(evidence: list[str]) -> float:
    """Score based on raw evidence count (lexical/content matches)."""
    return min(len(evidence), 10) / 10.0


def _semantic_signal(evidence: list[str], query: str) -> float:
    """Score based on evidence density. Generic — no intent branching."""
    if not evidence:
        return 0.0
    # Deterministic proxy: ratio of distinct evidence items (diversity)
    unique = len(set(evidence))
    return min(unique / max(len(evidence), 1), 1.0)


def _graph_signal(evidence: list[str]) -> float:
    """Score based on graph-topology evidence presence."""
    return 1.0 if evidence else 0.0


def _compute_signals(evidence: list[str], query: str) -> dict[str, float]:
    """Compute per-signal scores. Same pipeline for every query."""
    return {
        "text": _text_signal(evidence),
        "semantic": _semantic_signal(evidence, query),
        "graph": _graph_signal(evidence),
    }


def _aggregate(signals: dict[str, float], weights: dict[str, float]) -> float:
    """Weighted sum of signal scores."""
    return sum(signals[k] * weights[k] for k in weights)


def score_candidates(
    fused: dict[str, dict[str, Any]],
    query: str,
    mode: str = "deterministic",
) -> list[dict[str, Any]]:
    """Score and rank fused candidates with parser trace. Contract: C3, section 7.

    Args:
        fused: Output of ``fuse_candidates`` — mapping fragment_id to
               ``{"fragment_id": ..., "evidence": [...]}``.
        query: The search query (used identically regardless of content).
        mode: ``"deterministic"`` (stable ordering) or ``"stochastic"``
              (small random perturbation for exploration).

    Returns:
        Ranked list of dicts, each with ``fragment_id``, ``score``, and
        ``trace`` containing ``mode``, ``signals``, and ``weights``.
    """
    weights = dict(_DEFAULT_WEIGHTS)

    results: list[dict[str, Any]] = []
    for fid, entry in fused.items():
        evidence = entry.get("evidence", [])
        signals = _compute_signals(evidence, query)
        score = _aggregate(signals, weights)

        if mode == "stochastic":
            # Deterministic seed per (fid, query) so repeated calls with
            # same inputs still vary across different queries.
            seed = int(hashlib.sha256(f"{fid}:{query}".encode()).hexdigest()[:8], 16)
            rng = random.Random(seed)
            score += rng.uniform(-0.05, 0.05)

        results.append(
            {
                "fragment_id": fid,
                "score": score,
                "trace": {
                    "mode": mode,
                    "signals": signals,
                    "weights": dict(weights),
                },
            }
        )

    # Sort descending by score; tie-break by fragment_id for determinism.
    results.sort(key=lambda r: (-r["score"], r["fragment_id"]))
    return results
