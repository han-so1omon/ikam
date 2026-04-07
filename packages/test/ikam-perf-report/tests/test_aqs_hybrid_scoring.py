from __future__ import annotations

from ikam_perf_report.benchmarks.aqs import summarize_aqs


def test_hybrid_aqs_uses_manual_reviewer_score_when_present():
    query = {
        "query_id": "q-1",
        "oracle": {"coverage": 0.9, "grounded_precision": 0.8},
        "review": {"relevance": 0.7, "fidelity": 0.6, "clarity": 0.8, "note": "Solid"},
    }

    summary = summarize_aqs([query])

    assert summary["query_scores"][0]["oracle_score"] == 0.85
    assert summary["query_scores"][0]["reviewer_score"] == 0.7
    assert summary["query_scores"][0]["aqs"] == 0.805
    assert summary["query_scores"][0]["review_mode"] == "manual"


def test_hybrid_aqs_defaults_reviewer_to_oracle_when_manual_review_missing():
    query = {
        "query_id": "q-1",
        "oracle": {"coverage": 0.75, "grounded_precision": 0.65},
        "review": None,
    }

    summary = summarize_aqs([query])

    assert summary["query_scores"][0]["oracle_score"] == 0.7
    assert summary["query_scores"][0]["reviewer_score"] == 0.7
    assert summary["query_scores"][0]["aqs"] == 0.7
    assert summary["query_scores"][0]["review_mode"] == "oracle-defaulted"


def test_query_scores_include_oracle_components_and_review_rubric_fields():
    query = {
        "query_id": "q-1",
        "oracle": {"coverage": 0.8, "grounded_precision": 0.6},
        "review": {"relevance": 0.75, "fidelity": 0.65, "clarity": 0.7, "note": "Needs tighter evidence links"},
    }

    summary = summarize_aqs([query])
    query_score = summary["query_scores"][0]

    assert query_score["oracle"] == {"coverage": 0.8, "grounded_precision": 0.6}
    assert query_score["review"] == {
        "relevance": 0.75,
        "fidelity": 0.65,
        "clarity": 0.7,
        "note": "Needs tighter evidence links",
    }
