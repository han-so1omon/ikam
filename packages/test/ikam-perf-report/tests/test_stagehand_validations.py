
from __future__ import annotations
import pytest
pytest.importorskip('playwright')

import pytest

from ikam_perf_report.benchmarks.stagehand_validations import (
    build_search_query_candidates,
    resolve_evaluation_case_id,
    validate_evaluation_report,
    validate_visual_pass_signals,
    validate_wiki_document,
)


def _valid_evaluation() -> dict:
    return {
        "report": {
            "compression": {
                "total_fragments": 120,
                "unique_fragments": 90,
                "total_bytes": 10000,
                "unique_bytes": 7000,
                "dedup_ratio": 0.3,
            },
            "entities": {"coverage": 0.82, "passed": True},
            "predicates": {"predicate_coverage": 0.78, "contradiction_coverage": 0.9, "passed": True},
            "exploration": {"mean_recall": 0.75, "passed": True},
            "query": {"mean_fact_coverage": 0.8, "mean_quality_score": 8.1, "passed": True},
            "passed": True,
        },
        "rendered": "Compression\nEntities\nPredicates\nExploration\nQuery\nOverall",
    }


def _valid_wiki() -> dict:
    return {
        "graph_id": "g-1",
        "run_id": "r-1",
        "sections": [
            {
                "section_id": "sec-1",
                "title": "Fragment Narrative",
                "generated_markdown": "This section explains fragment relations and semantic links.",
                "generation_provenance": {
                    "model_id": "openai/gpt-4o-mini",
                    "harness_id": "modelado.wiki_generation.v1",
                    "prompt_fingerprint": "abc",
                    "input_snapshot_hash": "def",
                    "generated_at": "2026-02-12T00:00:00Z",
                },
            }
        ],
        "ikam_breakdown": {
            "section_id": "sec-2",
            "title": "IKAM Breakdown",
            "generated_markdown": "IKAM breakdown for graph g-1 includes nodes, edges, and provenance.",
            "generation_provenance": {
                "model_id": "openai/gpt-4o-mini",
                "harness_id": "modelado.wiki_generation.v1",
                "prompt_fingerprint": "ghi",
                "input_snapshot_hash": "jkl",
                "generated_at": "2026-02-12T00:00:00Z",
            },
        },
    }


def test_validate_evaluation_report_accepts_complete_structure() -> None:
    payload = _valid_evaluation()
    assert validate_evaluation_report(payload)["status"] == "ok"


def test_validate_evaluation_report_rejects_missing_required_sections() -> None:
    payload = _valid_evaluation()
    payload["report"].pop("query")
    with pytest.raises(AssertionError, match="evaluation_report_missing_sections"):
        validate_evaluation_report(payload)


def test_validate_wiki_document_rejects_incomplete_payload() -> None:
    wiki = _valid_wiki()
    wiki["ikam_breakdown"]["generated_markdown"] = "short"
    with pytest.raises(AssertionError, match="wiki_incomplete"):
        validate_wiki_document(wiki)


def test_validate_wiki_document_accepts_complete_wiki() -> None:
    assert validate_wiki_document(_valid_wiki())["status"] == "ok"


def test_validate_visual_pass_signals_rejects_weak_visual_signal() -> None:
    with pytest.raises(AssertionError, match="graph_visual_quality_low"):
        validate_visual_pass_signals(
            render_stats={"unique_colors": 80, "dominant_color_ratio": 0.92, "luminance_stddev": 8.0},
            nodes_count=40,
            edges_count=20,
        )


def test_validate_visual_pass_signals_accepts_healthy_signal() -> None:
    result = validate_visual_pass_signals(
        render_stats={"unique_colors": 700, "dominant_color_ratio": 0.34, "luminance_stddev": 28.0},
        nodes_count=130,
        edges_count=95,
    )
    assert result["status"] == "ok"


def test_resolve_evaluation_case_id_prefers_explicit_case() -> None:
    assert resolve_evaluation_case_id(active_case_id="s-construction-v01", explicit_case_id="s-local-retail-v01") == "s-local-retail-v01"


def test_resolve_evaluation_case_id_falls_back_to_active_case() -> None:
    assert resolve_evaluation_case_id(active_case_id="s-construction-v01", explicit_case_id="") == "s-construction-v01"


def test_build_search_query_candidates_adds_defaults_and_dedupes() -> None:
    values = build_search_query_candidates(primary_query="revenue", fallback_queries_csv="margin, revenue, forecast")
    assert values[0] == "revenue"
    assert "margin" in values
    assert "forecast" in values
    assert len(values) == len(set(values))
