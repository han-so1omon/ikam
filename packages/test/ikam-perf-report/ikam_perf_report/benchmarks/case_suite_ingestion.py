from __future__ import annotations

import asyncio
import json
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from uuid import uuid4

from ikam.forja.cas import cas_fragment
from ikam.forja.enricher import ENTITY_MIME, EntityRelationEnricher
from ikam.forja.normalizer import SemanticNormalizer
from ikam.fragments import CONCEPT_MIME, RELATION_MIME, Fragment

from modelado import ikam_metrics
from modelado.db import connection_scope
from modelado.ikam_graph_schema import create_ikam_schema, truncate_ikam_tables
from modelado.ikam_staging_store import StagingStore

from ikam_perf_report.benchmarks.case_fixtures import available_case_ids, load_case_fixture
from ikam_perf_report.benchmarks.aqs import summarize_aqs
from ikam_perf_report.benchmarks.quality_signals import (
    build_query_evaluations,
    compute_lane_metrics,
    evaluate_commit_lane_gates,
)


@dataclass
class _MockLLMResult:
    output: str


class _MockNormalizerLLM:
    async def call_model(self, prompt: str, model: str, temperature: float) -> _MockLLMResult:
        tail = prompt.split("Extract atomic concepts from:", 1)[-1].strip()
        pieces = [part.strip() for part in tail.split(".") if part.strip()]
        concepts = [piece[:80] for piece in pieces[:8]]
        if not concepts:
            concepts = ["No concepts extracted"]
        return _MockLLMResult(json.dumps(concepts))


class _MockEnrichmentLLM:
    async def call_model(self, prompt: str, model: str, temperature: float) -> _MockLLMResult:
        tokens = ["Acme", "Contoso", "Mexico", "City", "strategy", "revenue", "margin"]
        entities = [token for token in tokens if token.lower() in prompt.lower()]
        if not entities:
            entities = ["EntityA", "EntityB"]
        relations = []
        if len(entities) >= 2:
            relations.append({"source": entities[0], "target": entities[1], "predicate": "semantic_link"})
        return _MockLLMResult(json.dumps({"entities": entities, "relations": relations}))


def _metric_value(metric: Any) -> float:
    value = getattr(metric, "_value", None)
    if value is None:
        return 0.0
    try:
        return float(value.get())
    except Exception:
        return 0.0


def _count(row: Any) -> int:
    if isinstance(row, tuple):
        return int(row[0])
    return int(next(iter(row.values())))


def _decompose_asset(project_id: str, file_name: str, mime_type: str, payload: bytes) -> list[Fragment]:
    artifact_id = f"{project_id}:{file_name.replace('/', '_')}"
    effective_mime = mime_type or "application/octet-stream"
    if effective_mime.startswith("text/"):
        text = payload.decode("utf-8", errors="replace")
        return [cas_fragment({"text": text, "artifact_id": artifact_id}, effective_mime)]
    return [
        cas_fragment(
            {"artifact_id": artifact_id, "bytes_b64": payload[:1024].hex(), "truncated": len(payload) > 1024},
            effective_mime,
        )
    ]


async def _normalize_fragments(fragments: list[Fragment]) -> list[Fragment]:
    normalizer = SemanticNormalizer(ai_client=_MockNormalizerLLM())
    normalized: list[Fragment] = []
    for fragment in fragments:
        mime = fragment.mime_type or ""
        if mime == RELATION_MIME or mime == CONCEPT_MIME:
            continue
        # Extract text from value dict
        text = ""
        if isinstance(fragment.value, dict):
            text = fragment.value.get("text", "")
        if not text:
            continue
        normalized.extend(await normalizer.normalize(fragment))
    return normalized


async def _enrich_fragments(fragments: list[Fragment]) -> list[Fragment]:
    enricher = EntityRelationEnricher(ai_client=_MockEnrichmentLLM())
    enriched: list[Fragment] = []
    for fragment in fragments:
        mime = fragment.mime_type or ""
        if mime in (RELATION_MIME, ENTITY_MIME):
            continue
        try:
            enriched.extend(await enricher.enrich(fragment))
        except Exception:
            continue  # Mock LLM can fail on edge-case fragments; skip in perf report
    return enriched


def _build_graph_edges(all_fragments: list[Fragment]) -> list[dict[str, str]]:
    edges: list[dict[str, str]] = []
    for fragment in all_fragments:
        if fragment.mime_type != RELATION_MIME:
            continue
        value = fragment.value if isinstance(fragment.value, dict) else {}
        groups = value.get("binding_groups") if isinstance(value, dict) else None
        if not isinstance(groups, list):
            continue
        for group in groups:
            if not isinstance(group, dict):
                continue
            slots = group.get("slots")
            if not isinstance(slots, list):
                continue
            source: str | None = None
            targets: list[str] = []
            for slot in slots:
                if not isinstance(slot, dict):
                    continue
                fid = slot.get("fragment_id")
                key = slot.get("slot")
                if not (isinstance(fid, str) and fid):
                    continue
                if key == "source" and source is None:
                    source = fid
                else:
                    targets.append(fid)
            if not source and targets:
                source = targets[0]
                targets = targets[1:]
            if not source:
                continue
            for target in targets:
                edges.append({"source": source, "target": target, "label": "semantic_link"})
    return edges


def _to_mermaid(case_id: str, fragments: list[Fragment], edges: list[dict[str, str]]) -> str:
    lines = ["graph TD"]
    for fragment in fragments:
        node_id = (fragment.cas_id or "unknown").replace("-", "_")[:16]
        mime = fragment.mime_type or "application/octet-stream"
        lines.append(f"  {node_id}[\"{mime}\"]")
    for edge in edges:
        src = edge["source"].replace("-", "_")[:16]
        dst = edge["target"].replace("-", "_")[:16]
        lines.append(f"  {src} -->|{edge['label']}| {dst}")
    lines.append(f"  subgraph case_{case_id.replace('-', '_')}")
    lines.append("  end")
    return "\n".join(lines) + "\n"


def run_single_case_ingestion(case_id: str) -> dict[str, Any]:
    fixture = load_case_fixture(case_id)
    session_id = f"case-{case_id}-{uuid4().hex[:8]}"
    project_id = f"case-{case_id}-{uuid4().hex[:6]}"

    before_hits = _metric_value(ikam_metrics.cas_hits_total)
    before_misses = _metric_value(ikam_metrics.cas_misses_total)

    ingestion_size_bytes = sum(len(asset.payload) for asset in fixture.assets)

    with connection_scope() as cx:
        create_ikam_schema(cx)
        truncate_ikam_tables(cx)
        with cx.cursor() as cur:
            cur.execute("TRUNCATE ikam_staging_fragments")

        staging = StagingStore(cx)
        source_fragments: list[Fragment] = []
        for asset in fixture.assets:
            decomposed = _decompose_asset(project_id, asset.file_name, asset.mime_type, asset.payload)
            source_fragments.extend(decomposed)
            for fragment in decomposed:
                staging.stage_fragment(fragment, session_id)

        normalized = asyncio.run(_normalize_fragments(source_fragments))
        for fragment in normalized:
            staging.stage_fragment(fragment, session_id)

        enriched = asyncio.run(_enrich_fragments(normalized))
        for fragment in enriched:
            staging.stage_fragment(fragment, session_id)

        with cx.cursor() as cur:
            cur.execute("SELECT COUNT(*) FROM ikam_staging_fragments WHERE session_id = %s", (session_id,))
            staged_rows = _count(cur.fetchone())

        first_promoted = staging.promote_session(session_id)
        second_promoted = staging.promote_session(session_id)

        with cx.cursor() as cur:
            cur.execute(
                """
                SELECT COUNT(*)
                FROM ikam_fragments f
                JOIN ikam_staging_fragments s ON s.id = f.id
                WHERE s.session_id = %s
                """,
                (session_id,),
            )
            permanent_rows = _count(cur.fetchone())

    all_fragments = [*source_fragments, *normalized, *enriched]
    edges = _build_graph_edges(all_fragments)
    graph_nodes = len({fragment.cas_id for fragment in all_fragments if fragment.cas_id})
    graph_edges = len(edges)

    cas_events = len(all_fragments)
    cas_misses_delta = graph_nodes
    cas_hits_delta = max(0, cas_events - cas_misses_delta)
    total = cas_hits_delta + cas_misses_delta
    hit_rate = (cas_hits_delta / total) if total else 0.0

    promoted_ratio = round(permanent_rows / max(1, staged_rows), 4)
    lane_metrics = compute_lane_metrics(
        normalized_fragments=len(normalized),
        entity_fragments=sum(1 for f in enriched if f.mime_type == ENTITY_MIME),
        relation_fragments=sum(1 for f in enriched if f.mime_type == RELATION_MIME),
        graph_edges=graph_edges,
        cas_hit_rate=round(hit_rate, 4),
        second_promoted=second_promoted,
    )
    commit_gates = evaluate_commit_lane_gates(lane_metrics)
    query_evaluations = build_query_evaluations(
        case_id,
        graph_nodes=graph_nodes,
        graph_edges=graph_edges,
        asset_mime_types=[asset.mime_type for asset in fixture.assets],
        normalized_fragments=len(normalized),
        entity_fragments=sum(1 for f in enriched if f.mime_type == ENTITY_MIME),
        relation_fragments=sum(1 for f in enriched if f.mime_type == RELATION_MIME),
        promoted_ratio=promoted_ratio,
    )
    answer_quality = summarize_aqs(query_evaluations)

    return {
        "case_id": case_id,
        "domain": fixture.domain,
        "size_tier": fixture.size_tier,
        "asset_count": len(fixture.assets),
        "ingestion_size_bytes": ingestion_size_bytes,
        "staging": {
            "session_id": session_id,
            "rows": staged_rows,
        },
        "promotion": {
            "first_promoted": first_promoted,
            "second_promoted": second_promoted,
            "permanent_rows": permanent_rows,
        },
        "graph": {
            "nodes": graph_nodes,
            "edges": graph_edges,
            "mermaid": _to_mermaid(case_id, all_fragments, edges),
        },
        "processes": {
            "source_fragments": len(source_fragments),
            "normalized_fragments": len(normalized),
            "concept_fragments": sum(1 for f in normalized if f.mime_type == CONCEPT_MIME),
            "enriched_fragments": len(enriched),
            "entity_fragments": sum(1 for f in enriched if f.mime_type == ENTITY_MIME),
            "relation_fragments": sum(1 for f in enriched if f.mime_type == RELATION_MIME),
        },
        "dedup": {
            "cas_hits_delta": cas_hits_delta,
            "cas_misses_delta": cas_misses_delta,
            "cas_hit_rate": round(hit_rate, 4),
        },
        "storage_efficiency": {
            "rows_per_kb_ingested": round(staged_rows / max(1, ingestion_size_bytes / 1024.0), 4),
            "promoted_ratio": promoted_ratio,
        },
        "answer_quality": answer_quality,
        "lane_metrics": lane_metrics,
        "commit_lane_gates": commit_gates,
    }


def run_case_suite(case_ids: list[str] | None = None) -> list[dict[str, Any]]:
    selected = case_ids or available_case_ids()
    return [run_single_case_ingestion(case_id) for case_id in selected]


def write_case_suite_report(output_dir: Path, case_ids: list[str] | None = None) -> dict[str, Any]:
    output_dir.mkdir(parents=True, exist_ok=True)
    results = run_case_suite(case_ids)
    generated_at = datetime.now(timezone.utc).isoformat()
    summary = {"generated_at": generated_at, "case_count": len(results), "cases": []}

    for result in results:
        case_id = result["case_id"]
        case_json = output_dir / f"{case_id}.json"
        case_mmd = output_dir / f"{case_id}.mmd"
        case_json.write_text(json.dumps(result, indent=2), encoding="utf-8")
        case_mmd.write_text(result["graph"]["mermaid"], encoding="utf-8")
        summary["cases"].append(
            {
                "case_id": case_id,
                "json": str(case_json),
                "mermaid": str(case_mmd),
                "ingestion_size_bytes": result["ingestion_size_bytes"],
                "aqs": result["answer_quality"]["aqs"],
                "graph_nodes": result["graph"]["nodes"],
                "graph_edges": result["graph"]["edges"],
                "cas_hit_rate": result["dedup"]["cas_hit_rate"],
            }
        )

    summary_path = output_dir / "summary.json"
    summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    return summary
