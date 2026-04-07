from __future__ import annotations

from typing import Any


def _required_provenance_fields(payload: dict[str, Any]) -> bool:
    return all(
        isinstance(payload.get(key), str) and payload.get(key)
        for key in ("model_id", "harness_id", "prompt_fingerprint", "input_snapshot_hash")
    )


def resolve_evaluation_case_id(*, active_case_id: str, explicit_case_id: str | None) -> str:
    explicit = (explicit_case_id or "").strip()
    return explicit or active_case_id


def build_search_query_candidates(*, primary_query: str, fallback_queries_csv: str | None) -> list[str]:
    values: list[str] = []
    seen: set[str] = set()
    for raw in [primary_query, *(fallback_queries_csv or "").split(",")]:
        candidate = str(raw or "").strip()
        if not candidate:
            continue
        token = candidate.lower()
        if token in seen:
            continue
        seen.add(token)
        values.append(candidate)
    return values


def validate_evaluation_report(payload: dict[str, Any]) -> dict[str, Any]:
    report = payload.get("report")
    rendered = payload.get("rendered")
    if not isinstance(report, dict):
        raise AssertionError({"error": "evaluation_report_missing_report"})

    required_sections = {"compression", "entities", "predicates", "exploration", "query", "passed"}
    missing = sorted(section for section in required_sections if section not in report)
    if missing:
        raise AssertionError({"error": "evaluation_report_missing_sections", "missing": missing})

    if not isinstance(rendered, str) or not rendered.strip():
        raise AssertionError({"error": "evaluation_report_missing_rendered"})

    required_tokens = ("Compression", "Entities", "Predicates", "Exploration", "Query")
    missing_tokens = [token for token in required_tokens if token.lower() not in rendered.lower()]
    if missing_tokens:
        raise AssertionError({"error": "evaluation_report_rendered_missing_tokens", "missing": missing_tokens})

    return {"status": "ok", "required_sections": sorted(required_sections)}


def validate_wiki_document(wiki_doc: dict[str, Any], *, min_breakdown_len: int = 24) -> dict[str, Any]:
    if not isinstance(wiki_doc, dict):
        raise AssertionError({"error": "wiki_incomplete", "reason": "payload_not_dict"})

    if not isinstance(wiki_doc.get("graph_id"), str) or not wiki_doc.get("graph_id"):
        raise AssertionError({"error": "wiki_incomplete", "reason": "missing_graph_id"})

    if not isinstance(wiki_doc.get("run_id"), str) or not wiki_doc.get("run_id"):
        raise AssertionError({"error": "wiki_incomplete", "reason": "missing_run_id"})

    sections = wiki_doc.get("sections")
    if not isinstance(sections, list) or not sections:
        raise AssertionError({"error": "wiki_incomplete", "reason": "missing_sections"})

    for section in sections:
        if not isinstance(section, dict):
            raise AssertionError({"error": "wiki_incomplete", "reason": "invalid_section_payload"})
        section_markdown = section.get("generated_markdown")
        if not isinstance(section_markdown, str) or not section_markdown.strip():
            raise AssertionError({"error": "wiki_incomplete", "reason": "empty_section_content"})
        provenance = section.get("generation_provenance")
        if not isinstance(provenance, dict) or not _required_provenance_fields(provenance):
            raise AssertionError({"error": "wiki_incomplete", "reason": "invalid_section_provenance"})

    breakdown = wiki_doc.get("ikam_breakdown")
    if not isinstance(breakdown, dict):
        raise AssertionError({"error": "wiki_incomplete", "reason": "missing_ikam_breakdown"})

    if str(breakdown.get("title", "")).strip().lower() != "ikam breakdown":
        raise AssertionError({"error": "wiki_incomplete", "reason": "invalid_ikam_breakdown_title"})

    markdown = breakdown.get("generated_markdown")
    if not isinstance(markdown, str) or len(markdown.strip()) < min_breakdown_len:
        raise AssertionError({"error": "wiki_incomplete", "reason": "ikam_breakdown_content_too_short"})

    provenance = breakdown.get("generation_provenance")
    if not isinstance(provenance, dict) or not _required_provenance_fields(provenance):
        raise AssertionError({"error": "wiki_incomplete", "reason": "invalid_ikam_breakdown_provenance"})

    return {"status": "ok", "section_count": len(sections)}


def validate_visual_pass_signals(
    *,
    render_stats: dict[str, Any],
    nodes_count: int,
    edges_count: int,
    min_unique_colors: int = 200,
    max_dominant_ratio: float = 0.80,
    min_luminance_stddev: float = 12.0,
) -> dict[str, Any]:
    unique_colors = int(render_stats.get("unique_colors", 0) or 0)
    dominant_ratio = float(render_stats.get("dominant_color_ratio", 1.0) or 1.0)
    luminance_stddev = float(render_stats.get("luminance_stddev", 0.0) or 0.0)

    if nodes_count <= 0 or edges_count <= 0:
        raise AssertionError({"error": "graph_visual_quality_low", "reason": "empty_graph"})

    if unique_colors < min_unique_colors:
        raise AssertionError(
            {
                "error": "graph_visual_quality_low",
                "reason": "low_unique_colors",
                "actual": unique_colors,
                "expected_min": min_unique_colors,
            }
        )

    if dominant_ratio > max_dominant_ratio:
        raise AssertionError(
            {
                "error": "graph_visual_quality_low",
                "reason": "dominant_color_ratio_too_high",
                "actual": dominant_ratio,
                "expected_max": max_dominant_ratio,
            }
        )

    if luminance_stddev < min_luminance_stddev:
        raise AssertionError(
            {
                "error": "graph_visual_quality_low",
                "reason": "luminance_stddev_too_low",
                "actual": luminance_stddev,
                "expected_min": min_luminance_stddev,
            }
        )

    return {
        "status": "ok",
        "unique_colors": unique_colors,
        "dominant_color_ratio": dominant_ratio,
        "luminance_stddev": luminance_stddev,
    }


def validate_debug_pipeline_contract(payload: dict[str, Any]) -> dict[str, Any]:
    details = payload.get("details")
    if not isinstance(details, dict):
        raise AssertionError({"error": "debug_pipeline_missing_details"})
    debug_pipeline = details.get("debug_pipeline")
    if not isinstance(debug_pipeline, dict):
        raise AssertionError({"error": "debug_pipeline_missing_block"})

    required_keys = [
        "pipeline_id",
        "pipeline_run_id",
        "pipeline_steps",
        "env_handles",
        "step_trace",
        "candidate_discovery",
        "verification",
        "commit_summary",
        "retry_mode",
        "injection_used",
        "retry_reason",
    ]
    missing = [key for key in required_keys if key not in debug_pipeline]
    if missing:
        raise AssertionError({"error": "debug_pipeline_missing_keys", "missing": missing})

    commit_summary = debug_pipeline.get("commit_summary")
    if not isinstance(commit_summary, dict) or "ir_unmatched_count" not in commit_summary:
        raise AssertionError({"error": "debug_pipeline_missing_ir_unmatched_count"})

    return {"status": "ok", "required_keys": required_keys}


def parse_agentic_footer(text: str) -> dict[str, list[str]]:
    sections = {"observed": [], "not_observed": [], "uncertain": []}
    for raw_line in str(text or "").splitlines():
        line = raw_line.strip()
        if not line:
            continue
        lower = line.lower()
        if lower.startswith("observed:"):
            sections["observed"] = [item.strip() for item in line.split(":", 1)[1].split(",") if item.strip()]
        elif lower.startswith("not observed:"):
            sections["not_observed"] = [item.strip() for item in line.split(":", 1)[1].split(",") if item.strip()]
        elif lower.startswith("uncertain:"):
            sections["uncertain"] = [item.strip() for item in line.split(":", 1)[1].split(",") if item.strip()]
    return sections


def classify_agentic_mismatch(*, deterministic: dict[str, Any], agentic: dict[str, list[str]]) -> dict[str, Any]:
    mismatches: list[str] = []
    if bool(deterministic.get("retry_boundary")):
        not_observed = " ".join(agentic.get("not_observed", [])).lower()
        if "retry boundary" in not_observed:
            mismatches.append("retry_boundary_contradiction")

    if mismatches:
        raise AssertionError({"error": "agentic_mismatch", "mismatches": mismatches})
    return {"status": "ok", "mismatches": []}
