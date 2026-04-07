from __future__ import annotations

from typing import Any


def _clamp(score: float) -> float:
    return round(min(1.0, max(0.0, score)), 3)


def _oracle_score(oracle: dict[str, Any]) -> float:
    coverage = float(oracle.get("coverage", 0.0))
    grounded_precision = float(oracle.get("grounded_precision", 0.0))
    return _clamp((coverage + grounded_precision) / 2)


def _review_score(review: dict[str, Any] | None, oracle_score: float) -> tuple[float, str, dict[str, Any]]:
    if not review:
        return oracle_score, "oracle-defaulted", {
            "relevance": oracle_score,
            "fidelity": oracle_score,
            "clarity": oracle_score,
            "note": "Reviewer score defaulted to oracle score",
        }

    relevance = float(review.get("relevance", 0.0))
    fidelity = float(review.get("fidelity", 0.0))
    clarity = float(review.get("clarity", 0.0))
    note = str(review.get("note", "")).strip()
    reviewer = _clamp((relevance + fidelity + clarity) / 3)
    return reviewer, "manual", {
        "relevance": _clamp(relevance),
        "fidelity": _clamp(fidelity),
        "clarity": _clamp(clarity),
        "note": note,
    }


def summarize_aqs(query_evaluations: list[dict[str, Any]]) -> dict[str, Any]:
    query_scores: list[dict[str, Any]] = []
    manual_count = 0

    for index, query in enumerate(query_evaluations):
        query_id = str(query.get("query_id") or f"query-{index + 1}")
        oracle = dict(query.get("oracle") or {})
        oracle_score = _oracle_score(oracle)

        reviewer_score, review_mode, review = _review_score(query.get("review"), oracle_score)
        if review_mode == "manual":
            manual_count += 1

        aqs = _clamp((0.70 * oracle_score) + (0.30 * reviewer_score))
        query_scores.append(
            {
                "query_id": query_id,
                "oracle": {
                    "coverage": _clamp(float(oracle.get("coverage", 0.0))),
                    "grounded_precision": _clamp(float(oracle.get("grounded_precision", 0.0))),
                },
                "review": review,
                "oracle_score": oracle_score,
                "reviewer_score": reviewer_score,
                "aqs": aqs,
                "review_mode": review_mode,
            }
        )

    if not query_scores:
        return {
            "aqs": 0.0,
            "oracle_score": 0.0,
            "reviewer_score": 0.0,
            "review_mode": "oracle-defaulted",
            "review_coverage": 0.0,
            "query_scores": [],
        }

    run_oracle = _clamp(sum(item["oracle_score"] for item in query_scores) / len(query_scores))
    run_reviewer = _clamp(sum(item["reviewer_score"] for item in query_scores) / len(query_scores))
    run_aqs = _clamp(sum(item["aqs"] for item in query_scores) / len(query_scores))
    run_mode = "manual" if manual_count else "oracle-defaulted"
    review_coverage = _clamp(manual_count / len(query_scores))

    return {
        "aqs": run_aqs,
        "oracle_score": run_oracle,
        "reviewer_score": run_reviewer,
        "review_mode": run_mode,
        "review_coverage": review_coverage,
        "query_scores": query_scores,
    }
