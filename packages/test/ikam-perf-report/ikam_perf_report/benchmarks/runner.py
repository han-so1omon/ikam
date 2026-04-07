from __future__ import annotations

import asyncio
import json
import os
import re
from datetime import datetime, timezone
import importlib
from pathlib import Path
from time import perf_counter
from typing import Any, Dict
from uuid import uuid4

from fastapi import HTTPException
from ikam.forja.contracts import ExtractedEntity, ExtractedRelation, stable_entity_key, stable_relation_key
from ikam.forja.cas import cas_fragment
from ikam.forja.enricher import ENTITY_MIME, EntityRelationEnricher
from ikam.forja.normalizer import SemanticNormalizer
from ikam.ingest_uploaded import ingest_uploaded_file
from ikam.fragments import CONCEPT_MIME, RELATION_MIME, Fragment
from ikam.oraculo.composer import Evaluator
from ikam.oraculo.spec import OracleSpec
from modelado.oraculo.persistent_graph_state import PersistentGraphState
from modelado.oraculo.unified_bridge import UnifiedCallModelClient, UnifiedJudge

from modelado.db import connection_scope
from modelado.ikam_graph_schema import create_ikam_schema
from modelado.ikam_staging_store import StagingStore
from modelado import ikam_metrics

from ikam_perf_report.benchmarks.case_fixtures import (
    available_case_ids,
    load_case_fixture,
    parse_case_ids,
    validate_case_ids,
)
from ikam_perf_report.benchmarks.aqs import summarize_aqs
from ikam_perf_report.benchmarks.quality_signals import (
    build_commit_receipt,
    build_query_evaluations,
    compute_lane_metrics,
    evaluate_commit_lane_gates,
)
from ikam_perf_report.benchmarks.debug_models import DebugRunState, DebugStepEvent
from ikam_perf_report.benchmarks.evaluation_payload import serialize_evaluation_report
from ikam_perf_report.benchmarks.ikam_flow import fragments_to_graph
from ikam_perf_report.benchmarks.semantic_pipeline import run_semantic_pipeline
from ikam_perf_report.benchmarks.store import (
    BenchmarkRunRecord,
    EnrichmentItem,
    EnrichmentRun,
    GraphSnapshot,
    STORE,
)
from ikam_perf_report.decision_trace.collector import DecisionTraceCollector


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


async def _normalize_fragments(fragments: list[Any], *, ai_client: UnifiedCallModelClient) -> list[Fragment]:
    normalizer = SemanticNormalizer(ai_client=ai_client)
    normalized: list[Fragment] = []
    for fragment in fragments:
        mime = getattr(fragment, "mime_type", "") or ""
        if mime == RELATION_MIME:
            continue
        if not mime.startswith("text/"):
            continue
        normalized.extend(await normalizer.normalize(fragment))
    return normalized


async def _enrich_fragments(fragments: list[Fragment], *, ai_client: UnifiedCallModelClient) -> list[Fragment]:
    enricher = EntityRelationEnricher(ai_client=ai_client)
    enriched: list[Fragment] = []
    for fragment in fragments:
        enriched.extend(await enricher.enrich(fragment))
    return enriched


def run_staging_normalize_promote_enrich_poc() -> Dict[str, Any]:
    session_id = f"poc-{uuid4().hex[:10]}"
    artifact_id = f"poc-artifact-{uuid4().hex[:8]}"
    source_text = (
        "Acme reports stronger revenue quality in Mexico City. "
        "Contoso contributes distribution support and gross-margin gains. "
        "The strategy links market expansion with operating leverage."
    )

    before_hits = _metric_value(ikam_metrics.cas_hits_total)
    before_misses = _metric_value(ikam_metrics.cas_misses_total)

    ai_client = UnifiedCallModelClient.from_env()

    with connection_scope() as cx:
        create_ikam_schema(cx)
        staging = StagingStore(cx)

        source_fragments = [
            cas_fragment(
                {"text": source_text, "artifact_id": artifact_id},
                "text/markdown",
            )
        ]
        for fragment in source_fragments:
            staging.stage_fragment(fragment, session_id)

        normalized = asyncio.run(_normalize_fragments(source_fragments, ai_client=ai_client))
        for fragment in normalized:
            staging.stage_fragment(fragment, session_id)

        enriched = asyncio.run(_enrich_fragments(normalized, ai_client=ai_client))
        for fragment in enriched:
            staging.stage_fragment(fragment, session_id)

        with cx.cursor() as cur:
            cur.execute(
                """
                SELECT COUNT(*)
                FROM ikam_staging_fragments
                WHERE session_id = %s
                """,
                (session_id,),
            )
            staging_count = _count(cur.fetchone())

            cur.execute(
                """
                SELECT COUNT(*)
                FROM ikam_staging_fragments
                WHERE session_id = %s AND (size <= 0 OR mime_type IS NULL OR id IS NULL)
                """,
                (session_id,),
            )
            invalid_rows = _count(cur.fetchone())

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
            promoted_count = _count(cur.fetchone())

    after_hits = _metric_value(ikam_metrics.cas_hits_total)
    after_misses = _metric_value(ikam_metrics.cas_misses_total)
    hit_delta = max(0, int(after_hits - before_hits))
    miss_delta = max(0, int(after_misses - before_misses))
    total_delta = hit_delta + miss_delta
    hit_rate = (hit_delta / total_delta) if total_delta else 0.0

    entity_count = sum(1 for fragment in enriched if fragment.mime_type == ENTITY_MIME)
    relation_count = sum(1 for fragment in enriched if fragment.mime_type == RELATION_MIME)
    concept_count = sum(1 for fragment in normalized if fragment.mime_type == CONCEPT_MIME)

    return {
        "staging": {
            "session_id": session_id,
            "row_count": staging_count,
            "valid": invalid_rows == 0,
            "invalid_rows": invalid_rows,
        },
        "normalization": {
            "concept_fragments": concept_count,
        },
        "enrichment": {
            "entity_fragments": entity_count,
            "relation_fragments": relation_count,
        },
        "promotion": {
            "first_promoted": first_promoted,
            "second_promoted": second_promoted,
            "permanent_count": promoted_count,
        },
        "metrics": {
            "cas_hits_delta": hit_delta,
            "cas_misses_delta": miss_delta,
            "cas_hit_rate": round(hit_rate, 4),
        },
    }


def _stage_record(name: str, started_at: datetime, ended_at: datetime) -> Dict[str, Any]:
    duration_ms = max(1, int((ended_at - started_at).total_seconds() * 1000))
    return {
        "id": f"stage-{uuid4().hex}",
        "stage_name": name,
        "started_at": started_at.isoformat(),
        "ended_at": ended_at.isoformat(),
        "duration_ms": duration_ms,
    }


def _initialize_debug_workflow_for_run(
    *,
    run_id: str,
    project_id: str,
    operation_id: str,
    source_bytes: bytes,
    mime_type: str,
    artifact_id: str,
    asset_manifest: list[dict[str, Any]] | None = None,
    asset_payloads: list[dict[str, Any]] | None = None,
    pipeline_id: str | None = None,
) -> None:
    pipeline_id = pipeline_id or "compression-rerender/v1"
    pipeline_run_id = run_id
    env_type = "dev"
    env_id = f"dev-{run_id[:8]}"
    STORE.create_debug_run_state(
        DebugRunState(
            run_id=run_id,
            pipeline_id=pipeline_id,
            pipeline_run_id=pipeline_run_id,
            project_id=project_id,
            operation_id=operation_id,
            env_type=env_type,
            env_id=env_id,
            execution_mode="manual",
            execution_state="paused",
            current_step_name="init.initialize",
            current_attempt_index=1,
        )
    )
    now = datetime.now(timezone.utc).isoformat()
    STORE.append_debug_event(
        DebugStepEvent(
            event_id=f"ev-{uuid4().hex}",
            run_id=run_id,
            pipeline_id=pipeline_id,
            pipeline_run_id=pipeline_run_id,
            project_id=project_id,
            operation_id=operation_id,
            env_type=env_type,
            env_id=env_id,
            step_name="init.initialize",
            step_id=f"step-{uuid4().hex[:10]}",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at=now,
            ended_at=now,
            duration_ms=1,
            metrics={
                "trigger": "run_button",
                "immediate_stream": True,
                "details": {
                    "artifact_id": artifact_id,
                    "mime_type": mime_type,
                    "source_bytes": len(source_bytes),
                },
            },
            error=None,
        )
    )
    STORE.set_debug_runtime_context(
        run_id,
        {
            "source_bytes": source_bytes,
            "mime_type": mime_type,
            "artifact_id": artifact_id,
            "asset_manifest": list(asset_manifest or []),
            "asset_payloads": list(asset_payloads or []),
            "step_outputs": {},
        },
    )


def _build_asset_manifest(*, project_id: str, assets: list[Any]) -> list[dict[str, Any]]:
    manifest: list[dict[str, Any]] = []
    for index, asset in enumerate(assets):
        file_name = str(getattr(asset, "file_name", f"asset-{index}"))
        mime_type = str(getattr(asset, "mime_type", "application/octet-stream"))
        payload = getattr(asset, "payload", b"")
        size_bytes = len(payload) if isinstance(payload, (bytes, bytearray)) else 0
        manifest.append(
            {
                "artifact_id": f"{project_id}:{file_name.replace('/', '_')}",
                "filename": file_name,
                "mime_type": mime_type,
                "size_bytes": size_bytes,
                "role": "source",
                "included": True,
                "exclusion_reason": None,
            }
        )
    return manifest


def _build_asset_payloads(*, project_id: str, assets: list[Any]) -> list[dict[str, Any]]:
    payloads: list[dict[str, Any]] = []
    for index, asset in enumerate(assets):
        file_name = str(getattr(asset, "file_name", f"asset-{index}"))
        mime_type = str(getattr(asset, "mime_type", "application/octet-stream"))
        payload = getattr(asset, "payload", b"")
        if not isinstance(payload, (bytes, bytearray)):
            payload = bytes(str(payload), encoding="utf-8")
        payloads.append(
            {
                "artifact_id": f"{project_id}:{file_name.replace('/', '_')}",
                "filename": file_name,
                "mime_type": mime_type,
                "payload": bytes(payload),
            }
        )
    return payloads


def _decode_text(payload: bytes) -> str:
    return payload.decode("utf-8", errors="ignore")


def _is_text_file(file_name: str, mime_type: str) -> bool:
    return file_name.endswith((".md", ".txt", ".json")) or mime_type.startswith("text/")


def _decompose_asset(project_id: str, file_name: str, mime_type: str, payload: bytes) -> tuple[list[Any], str | None]:
    artifact_id = f"{project_id}:{file_name.replace('/', '_')}"
    if _is_text_file(file_name, mime_type):
        text = payload.decode("utf-8", errors="replace")
        return [cas_fragment({"text": text, "artifact_id": artifact_id}, mime_type or "text/plain")], text
    _, fragment = ingest_uploaded_file(payload, mime_type=mime_type, artifact_id=artifact_id, title=file_name)
    return [fragment], None


def _semantic_input(assets: list[Any]) -> str:
    chunks: list[str] = []
    for asset in assets:
        if _is_text_file(asset.file_name, asset.mime_type):
            chunks.append(_decode_text(asset.payload)[:1200])
        if len(chunks) >= 5:
            break
    return "\n\n".join(chunks) or "No textual fixture content"


def _fragment_id(fragment: Any) -> str:
    return getattr(fragment, "cas_id", None) or getattr(fragment, "id", None) or ""


def _fragment_text(fragment: Any) -> str:
    value = getattr(fragment, "value", None)
    if isinstance(value, dict):
        text = value.get("text")
        if isinstance(text, str):
            return text
        concept = value.get("concept")
        if isinstance(concept, str):
            return concept
    if isinstance(value, str):
        return value
    return ""


def _asset_label(file_name: str) -> str:
    stem = file_name.rsplit("/", 1)[-1]
    stem = stem.rsplit(".", 1)[0] if "." in stem else stem
    return stem.replace("_", " ").replace("-", " ")


def _build_oraculo_graph(fixture_assets: list[Any], all_fragments: list[Any]) -> PersistentGraphState:
    """Build an InMemoryGraphState from case assets for Oráculo evaluation.

    Mirrors the _build_graph() logic in evaluations.py: decomposes text assets
    into fragments, extracts entities via regex, and builds co-occurrence relations.
    Only text assets are included — binary assets (xlsx, pptx, images) are excluded
    to keep the graph within LLM context limits.
    """
    _TEXT_EXTS = {".md", ".txt", ".csv", ".tsv"}
    gs = PersistentGraphState()

    source_entity_labels: list[tuple[str, list[str]]] = []
    semantic_chunks: list[str] = []
    for asset in fixture_assets:
        ext = os.path.splitext(asset.file_name)[1].lower()
        is_text = (
            (asset.mime_type and asset.mime_type.startswith("text/"))
            or ext in _TEXT_EXTS
        )
        if not is_text:
            continue
        try:
            text = asset.payload.decode("utf-8")
        except UnicodeDecodeError:
            continue

        # Create one semantic text fragment per text asset for graph seeding
        fragments = [cas_fragment({"text": text, "artifact_id": asset.file_name}, "text/markdown")]
        for frag in fragments:
            gs.add_fragment(frag)

        candidates = re.findall(r"\b[A-Z][a-zA-Z]+(?:\s+[A-Z][a-zA-Z]+)*\b", text)
        seen: set[str] = set()
        labels: list[str] = []
        for raw in candidates:
            canonical = " ".join(raw.strip().lower().split())
            if not canonical or canonical in seen or len(canonical) < 3:
                continue
            seen.add(canonical)
            entity = ExtractedEntity(
                label=raw.strip(),
                canonical_label=canonical,
                source_fragment_id=asset.file_name,
                entity_key=stable_entity_key(asset.file_name, raw),
            )
            gs.add_entity(entity)
            labels.append(canonical)
        source_entity_labels.append((asset.file_name, labels))
        semantic_chunks.append(text[:1200])

    semantic_payload = run_semantic_pipeline("\n\n".join(semantic_chunks) or "No textual fixture content")

    for item in semantic_payload.get("entities", []):
        label = str(item.get("label") or "").strip()
        if not label:
            continue
        canonical = " ".join(label.lower().split())
        if gs.entity_by_name(canonical):
            continue
        gs.add_entity(
            ExtractedEntity(
                label=label,
                canonical_label=canonical,
                source_fragment_id="semantic-pipeline",
                entity_key=stable_entity_key("semantic-pipeline", label),
            )
        )

    relation_added = False
    for item in semantic_payload.get("relations", []):
        source_label = str(item.get("source_label") or item.get("source") or "").strip()
        target_label = str(item.get("target_label") or item.get("target") or "").strip()
        predicate = str(item.get("kind") or item.get("predicate") or "").strip().lower()
        if not source_label or not target_label or not predicate:
            continue
        src_entity = gs.entity_by_name(source_label)
        dst_entity = gs.entity_by_name(target_label)
        if src_entity is None or dst_entity is None:
            continue
        gs.add_relation(
            ExtractedRelation(
                predicate=predicate,
                source_label=src_entity.label,
                target_label=dst_entity.label,
                source_entity_key=src_entity.entity_key,
                target_entity_key=dst_entity.entity_key,
                relation_key=stable_relation_key("semantic-pipeline", predicate, src_entity.label, dst_entity.label),
            )
        )
        relation_added = True

    if not relation_added:
        for source_id, labels in source_entity_labels:
            surviving = []
            for label in labels[:10]:
                entity = gs.entity_by_name(label)
                if entity:
                    surviving.append(entity)
            for i, src_e in enumerate(surviving):
                for tgt_e in surviving[i + 1 : i + 4]:
                    gs.add_relation(
                        ExtractedRelation(
                            predicate="co-occurs",
                            source_label=src_e.label,
                            target_label=tgt_e.label,
                            source_entity_key=src_e.entity_key,
                            target_entity_key=tgt_e.entity_key,
                            relation_key=stable_relation_key(source_id, "co-occurs", src_e.label, tgt_e.label),
                        )
                    )

    return gs


def _find_oracle_path(case_id: str) -> Path | None:
    """Locate oracle.json for a case, returning None if it doesn't exist."""
    from ikam_perf_report.benchmarks.case_fixtures import _cases_root
    oracle_path = _cases_root() / case_id / "oracle.json"
    return oracle_path if oracle_path.exists() else None


def _run_oraculo_evaluation(
    case_id: str,
    fixture_assets: list[Any],
    all_fragments: list[Any],
) -> Dict[str, Any] | None:
    """Run Oráculo evaluation if oracle fixture exists. Returns dict or None."""
    oracle_path = _find_oracle_path(case_id)
    if oracle_path is None:
        return None

    spec = OracleSpec.from_json(str(oracle_path))
    graph = _build_oraculo_graph(fixture_assets, all_fragments)
    judge_model = os.getenv("LLM_JUDGE_MODEL", "").strip() or os.getenv("LLM_MODEL", "").strip()
    judge = UnifiedJudge(model=judge_model)
    evaluator = Evaluator(judge=judge)
    report = evaluator.evaluate_all(graph, spec)
    return serialize_evaluation_report(report)


def _select_node_for_relation(label: str, nodes: list[dict[str, Any]]) -> str | None:
    normalized = str(label or "").strip().lower()
    if not normalized:
        return None
    for node in nodes:
        node_label = str(node.get("label") or "").strip().lower()
        if node_label == normalized:
            return str(node.get("id") or "") or None
    for node in nodes:
        node_label = str(node.get("label") or "").strip().lower()
        if normalized in node_label or node_label in normalized:
            return str(node.get("id") or "") or None
    return None


def _select_fragment_node_for_relation(
    label: str,
    fragments: list[Any],
    node_ids: set[str],
) -> str | None:
    normalized = str(label or "").strip().lower()
    if not normalized:
        return None
    for fragment in fragments:
        fragment_id = _fragment_id(fragment)
        if not fragment_id or fragment_id not in node_ids:
            continue
        text = _fragment_text(fragment).lower()
        if normalized in text:
            return fragment_id
    return None


def run_benchmark(case_ids: str | None = None, reset: bool = False, include_evaluation: bool = True, pipeline_id: str | None = None) -> Dict[str, Any]:
    """Prepare-only benchmark runner.

    Loads fixtures, stores source_bytes and runtime context, creates
    DebugRunState at prepare_case, and returns immediately (<100ms).
    All computation (decompose, lift, embed, etc.) happens later via
    execute_step() handlers invoked by next_step/resume controls.
    """
    if reset:
        reset_module = importlib.import_module("modelado.ikam_reset")
        reset_ikam_graph_state = getattr(reset_module, "reset_ikam_graph_state")
        reset_ikam_graph_state()
        STORE.reset()

    requested_case_ids = parse_case_ids(case_ids)
    valid_case_ids, invalid_case_ids = validate_case_ids(requested_case_ids)
    if invalid_case_ids:
        raise HTTPException(
            status_code=400,
            detail={
                "error": "invalid_case_ids",
                "invalid": invalid_case_ids,
                "available": available_case_ids(),
            },
        )

    runs: list[Dict[str, Any]] = []
    for case_id in valid_case_ids:
        project_id = STORE.next_project_id(case_id)
        fixture = load_case_fixture(case_id)
        run_id = f"run-{uuid4().hex}"
        operation_id = f"op-{run_id}"

        source_bytes = _semantic_input(fixture.assets).encode("utf-8")
        artifact_id = f"{project_id}:{case_id}"

        _initialize_debug_workflow_for_run(
            run_id=run_id,
            project_id=project_id,
            operation_id=operation_id,
            source_bytes=source_bytes,
            mime_type="text/markdown",
            artifact_id=artifact_id,
            asset_manifest=_build_asset_manifest(project_id=project_id, assets=fixture.assets),
            asset_payloads=_build_asset_payloads(project_id=project_id, assets=fixture.assets),
            pipeline_id=pipeline_id,
        )

        project_payload = {
            "case_id": case_id,
            "project_id": project_id,
            "domain": fixture.domain,
            "size_tier": fixture.size_tier,
            "asset_count": len(fixture.assets),
        }

        record = BenchmarkRunRecord(
            run_id=run_id,
            project_id=project_id,
            case_id=case_id,
            stages=[],
            decisions=[],
            project=project_payload,
            graph=GraphSnapshot(graph_id=project_id),
            semantic=None,
            answer_quality=None,
            commit_receipt=None,
            evaluation=None,
        )
        STORE.add_run(record)

        debug_state = STORE.get_debug_run_state(run_id)
        runs.append(
            {
                "run_id": run_id,
                "project_id": project_id,
                "graph_id": project_id,
                "case_id": case_id,
                "project": project_payload,
                "debug_state": {
                    "current_step_name": debug_state.current_step_name,
                    "execution_state": debug_state.execution_state,
                } if debug_state else None,
            }
        )

    return {
        "requested_case_ids": requested_case_ids,
        "runs": runs,
    }


def run_benchmark_legacy(case_ids: str | None = None, reset: bool = False, include_evaluation: bool = True) -> Dict[str, Any]:
    """Legacy full-computation benchmark runner.

    Preserved for incremental test migration. Runs the complete pipeline
    synchronously: semantic → decompose → graph → quality → enrichment → evaluation.
    Tests that need full computation should call this directly until they
    are migrated to the step-based execution model.
    """
    if reset:
        reset_module = importlib.import_module("modelado.ikam_reset")
        reset_ikam_graph_state = getattr(reset_module, "reset_ikam_graph_state")
        reset_ikam_graph_state()
        STORE.reset()

    requested_case_ids = parse_case_ids(case_ids)
    valid_case_ids, invalid_case_ids = validate_case_ids(requested_case_ids)
    if invalid_case_ids:
        raise HTTPException(
            status_code=400,
            detail={
                "error": "invalid_case_ids",
                "invalid": invalid_case_ids,
                "available": available_case_ids(),
            },
        )

    runs: list[Dict[str, Any]] = []
    for case_id in valid_case_ids:
        project_id = STORE.next_project_id(case_id)
        fixture = load_case_fixture(case_id)
        collector = DecisionTraceCollector()

        semantic_start = datetime.now(timezone.utc)
        semantic = run_semantic_pipeline(_semantic_input(fixture.assets))
        semantic_end = datetime.now(timezone.utc)

        decompose_start = datetime.now(timezone.utc)
        decompose_perf_start = perf_counter()
        all_fragments: list[Any] = []
        labels_by_fragment_id: dict[str, str] = {}
        reconstructed_text_bytes = 0
        for asset in fixture.assets:
            fragments, reconstructed_text = _decompose_asset(
                project_id=project_id,
                file_name=asset.file_name,
                mime_type=asset.mime_type,
                payload=asset.payload,
            )
            all_fragments.extend(fragments)
            label = _asset_label(asset.file_name)
            for fragment in fragments:
                fragment_id = _fragment_id(fragment)
                if fragment_id:
                    labels_by_fragment_id[fragment_id] = label
            if reconstructed_text:
                reconstructed_text_bytes += len(reconstructed_text.encode("utf-8"))
        decompose_perf_end = perf_counter()
        decompose_end = datetime.now(timezone.utc)

        graph_start = datetime.now(timezone.utc)
        nodes, edges, manifests = fragments_to_graph(
            all_fragments,
            labels_by_fragment_id=labels_by_fragment_id,
            filenames_by_label={_asset_label(a.file_name): a.file_name for a in fixture.assets},
        )
        graph_end = datetime.now(timezone.utc)
        quality_start = datetime.now(timezone.utc)
        query_evaluations = build_query_evaluations(
            case_id,
            graph_nodes=len(nodes),
            graph_edges=len(edges),
            asset_mime_types=[asset.mime_type for asset in fixture.assets],
            normalized_fragments=len(all_fragments),
            entity_fragments=len(semantic.get("entities", [])),
            relation_fragments=len(semantic.get("relations", [])),
            promoted_ratio=1.0,
        )
        answer_quality = summarize_aqs(query_evaluations)
        quality_end = datetime.now(timezone.utc)

        enrichment_start = datetime.now(timezone.utc)
        run_id = f"run-{uuid4().hex}"
        operation_id = f"op-{run_id}"
        node_ids = [str(node.get("id")) for node in nodes if node.get("id")]
        node_id_set = set(node_ids)
        relation_items: list[EnrichmentItem] = []
        enrichment_id = STORE.next_enrichment_id(project_id)
        relation_candidates = semantic.get("relations", [])
        for index, relation in enumerate(relation_candidates, start=1):
            source = str(relation.get("source") or "")
            target = str(relation.get("target") or "")
            source_label = str(relation.get("source_label") or source)
            target_label = str(relation.get("target_label") or target)
            unresolved = False
            source_node = _select_node_for_relation(source_label, nodes)
            target_node = _select_node_for_relation(target_label, nodes)
            if not source_node:
                source_node = _select_fragment_node_for_relation(source_label, all_fragments, node_id_set)
            if not target_node:
                target_node = _select_fragment_node_for_relation(target_label, all_fragments, node_id_set)
            if source_node:
                source = source_node
            if target_node:
                target = target_node
            if source not in node_ids:
                source = node_ids[0] if node_ids else source
                unresolved = True
            if target not in node_ids:
                target = node_ids[-1] if node_ids else target
                unresolved = True
            relation_items.append(
                EnrichmentItem(
                    enrichment_id=enrichment_id,
                    run_id=run_id,
                    graph_id=project_id,
                    relation_id=str(relation.get("id") or f"relation-{uuid4().hex}"),
                    relation_kind=str(relation.get("kind") or "semantic_link"),
                    source=source,
                    target=target,
                    rationale=str(relation.get("rationale") or ""),
                    evidence=[str(item) for item in (relation.get("evidence") or []) if item],
                    status="staged",
                    sequence=index,
                    lane_mode="explore-graph",
                    unresolved=unresolved,
                )
            )
        lane_metrics = compute_lane_metrics(
            normalized_fragments=len(all_fragments),
            entity_fragments=len(semantic.get("entities", [])),
            relation_fragments=len(semantic.get("relations", [])),
            graph_edges=len(edges),
            cas_hit_rate=1.0,
            second_promoted=0,
        )
        commit_lane_gates = evaluate_commit_lane_gates(lane_metrics)
        commit_receipt = build_commit_receipt(
            case_id=case_id,
            mode="commit-strict",
            committed_fragment_ids=sorted(
                [
                    _fragment_id(fragment)
                    for fragment in all_fragments
                    if _fragment_id(fragment)
                ]
            ),
            edge_idempotency_keys=sorted(
                [
                    str(edge.get("id") or f"{edge.get('source')}->{edge.get('target')}:{edge.get('label')}")
                    for edge in edges
                ]
            ),
            unresolved_endpoints=[],
        )

        collector.record(
            step_index=1,
            decision_type="case_selection",
            inputs={"case_id": case_id},
            outputs={"project_id": project_id, "assets": len(fixture.assets)},
        )
        collector.record(
            step_index=2,
            decision_type="ikam_decompose",
            inputs={"assets": len(fixture.assets)},
            outputs={
                "fragments": len(all_fragments),
                "duration_ms": max(1, int((decompose_perf_end - decompose_perf_start) * 1000)),
            },
        )
        collector.record(
            step_index=3,
            decision_type="ikam_reconstruct",
            inputs={"case_id": case_id},
            outputs={"reconstructed_text_bytes": reconstructed_text_bytes},
        )

        if relation_items:
            sequence = 1
            try:
                sequence = int(enrichment_id.rsplit(":", 1)[-1])
            except Exception:
                sequence = 1

            STORE.add_enrichment_run(
                EnrichmentRun(
                    enrichment_id=enrichment_id,
                    run_id=run_id,
                    graph_id=project_id,
                    sequence=sequence,
                    lane_mode="explore-graph",
                    status="staged",
                    relation_count=len(relation_items),
                    unresolved_count=sum(1 for item in relation_items if item.unresolved),
                ),
                relation_items,
            )
        project_payload = {
            "case_id": case_id,
            "project_id": project_id,
            "domain": fixture.domain,
            "size_tier": fixture.size_tier,
            "asset_count": len(fixture.assets),
        }
        enrichment_end = datetime.now(timezone.utc)

        # Run Oráculo evaluation if requested and oracle fixture exists for this case
        evaluation = _run_oraculo_evaluation(case_id, fixture.assets, all_fragments) if include_evaluation else {"status": "skipped", "reason": "fast_debug_init"}
        if isinstance(evaluation, dict) and evaluation.get("status") != "skipped":
            details = evaluation.get("details")
            if not isinstance(details, dict):
                details = {}
                evaluation["details"] = details
            details["debug_pipeline"] = {
                "pipeline_id": "compression-rerender/v1",
                "pipeline_run_id": run_id,
            }

        debug_init_start = datetime.now(timezone.utc)
        _initialize_debug_workflow_for_run(
            run_id=run_id,
            project_id=project_id,
            operation_id=operation_id,
            source_bytes=_semantic_input(fixture.assets).encode("utf-8"),
            mime_type="text/markdown",
            artifact_id=f"{project_id}:{case_id}",
            asset_manifest=_build_asset_manifest(project_id=project_id, assets=fixture.assets),
            asset_payloads=_build_asset_payloads(project_id=project_id, assets=fixture.assets),
        )
        debug_init_end = datetime.now(timezone.utc)

        stages = [
            _stage_record("semantic_pipeline", semantic_start, semantic_end),
            _stage_record("decompose_artifacts", decompose_start, decompose_end),
            _stage_record("graph_conversion", graph_start, graph_end),
            _stage_record("quality_signals", quality_start, quality_end),
            _stage_record("enrichment_prep", enrichment_start, enrichment_end),
            _stage_record("debug_init", debug_init_start, debug_init_end),
        ]

        record = BenchmarkRunRecord(
            run_id=run_id,
            project_id=project_id,
            case_id=case_id,
            stages=stages,
            decisions=collector.to_dicts(),
            project=project_payload,
            graph=GraphSnapshot(graph_id=project_id, nodes=nodes, edges=edges, manifests=manifests, fragments=list(all_fragments)),
            semantic=semantic,
            answer_quality=answer_quality,
            commit_receipt=commit_receipt,
            evaluation=evaluation,
        )
        STORE.add_run(record)

        runs.append(
            {
                "run_id": run_id,
                "project_id": project_id,
                "graph_id": project_id,
                "case_id": case_id,
                "stages": stages,
                "decisions": collector.to_dicts(),
                "project": project_payload,
                "semantic": semantic,
                "answer_quality": answer_quality,
                "lane_metrics": lane_metrics,
                "commit_lane_gates": commit_lane_gates,
                "commit_receipt": commit_receipt,
                "evaluation": evaluation,
            }
        )

    return {
        "requested_case_ids": requested_case_ids,
        "runs": runs,
    }


def run_interacciones_benchmark(case_ids: str | None = None) -> Dict[str, Any]:
    selected = parse_case_ids(case_ids)
    collector = DecisionTraceCollector()
    stage_start = datetime.now(timezone.utc)
    fixture = load_case_fixture(selected[0])
    doc_payload = _semantic_input(fixture.assets)
    fragments = [
        cas_fragment(
            {"text": doc_payload, "artifact_id": f"artifact-{uuid4().hex}"},
            "text/markdown",
        )
    ]
    stage_end = datetime.now(timezone.utc)

    collector.record(
        step_index=1,
        decision_type="interacciones_artifact",
        inputs={"case_id": fixture.case_id},
        outputs={"fragments": len(fragments)},
    )

    return {
        "run_id": f"run-{uuid4().hex}",
        "stages": [_stage_record("interacciones_artifact", stage_start, stage_end)],
        "decisions": collector.to_dicts(),
        "project": {"case_id": fixture.case_id},
    }


def run_merge_benchmark(graph_ids: str, apply: bool = False) -> Dict[str, Any]:
    selected_graph_ids = [item.strip() for item in graph_ids.split(",") if item.strip()]
    if not selected_graph_ids:
        raise HTTPException(status_code=400, detail={"error": "graph_ids_required"})

    missing = [graph_id for graph_id in selected_graph_ids if not STORE.get_graph(graph_id)]
    if missing:
        raise HTTPException(
            status_code=400,
            detail={
                "error": "missing_graph_ids",
                "missing": missing,
            },
        )

    collector = DecisionTraceCollector()
    stage_start = datetime.now(timezone.utc)
    graphs = [STORE.get_graph(graph_id) for graph_id in selected_graph_ids]

    node_types: set[str] = set()
    for graph in graphs:
        if not graph:
            continue
        node_types.update(node.get("type", "unknown") for node in graph.nodes)

    proposed_edges = [
        {
            "op": "upsert",
            "source": selected_graph_ids[0],
            "target": selected_graph_ids[-1],
            "label": "merge_candidate",
            "properties": {"shared_node_types": sorted(node_types), "graph_ids": selected_graph_ids},
        }
    ]
    proposed_relational_fragments = [
        {
            "id": f"relation-{uuid4().hex}",
            "kind": "connection_fragment",
            "reasons": {"structural_path_compress": 1.0},
            "evidence": selected_graph_ids,
            "expands_to": proposed_edges,
        }
    ]
    stage_end = datetime.now(timezone.utc)

    synergy_risk = {
        "synergy": round(min(0.95, 0.3 + (len(node_types) / 20)), 2),
        "risk": round(min(0.9, 0.25 + (len(selected_graph_ids) / 20)), 2),
        "summary": "Projected overlap across selected graphs",
    }

    collector.record(
        step_index=1,
        decision_type="graph_merge",
        inputs={"graph_ids": selected_graph_ids},
        outputs={
            "synergy_risk": synergy_risk,
            "proposed_edges": len(proposed_edges),
            "proposed_relational_fragments": len(proposed_relational_fragments),
            "applied": apply,
        },
    )

    apply_result = None
    if apply:
        apply_result = STORE.apply_merge_updates(
            graph_ids=selected_graph_ids,
            proposed_edges=proposed_edges,
            proposed_relational_fragments=proposed_relational_fragments,
        )

    return {
        "merge_id": f"merge-{uuid4().hex}",
        "run_id": f"run-{uuid4().hex}",
        "stages": [_stage_record("graph_merge", stage_start, stage_end)],
        "decisions": collector.to_dicts(),
        "synergy_risk": synergy_risk,
        "graph_ids": selected_graph_ids,
        "proposed_edges": proposed_edges,
        "proposed_relational_fragments": proposed_relational_fragments,
        "applied": apply,
        "apply_result": apply_result,
    }
