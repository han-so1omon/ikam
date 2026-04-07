"""Evaluations API — run oráculo evaluator pipeline on a case fixture.

Exposes /evaluations/run which:
1. Loads the OracleSpec from the case fixture
2. Builds a graph from case assets
3. Runs the full evaluator (compression, entities, predicates, exploration, query)
4. Returns both structured report and rendered text
"""
from __future__ import annotations

import os

from fastapi import APIRouter, HTTPException

from ikam.forja.contracts import ExtractedEntity, ExtractedRelation, stable_entity_key, stable_relation_key
from ikam.forja.cas import cas_fragment
from ikam.oraculo.composer import Evaluator
from ikam.oraculo.spec import OracleSpec
from modelado.oraculo.persistent_graph_state import PersistentGraphState
from modelado.oraculo.unified_bridge import UnifiedJudge

from ikam_perf_report.benchmarks.case_fixtures import load_case_fixture
from ikam_perf_report.benchmarks.evaluation_payload import serialize_evaluation_report
from ikam_perf_report.benchmarks.semantic_pipeline import run_semantic_pipeline

router = APIRouter(prefix="/evaluations", tags=["evaluations"])


def _build_graph(case_id: str) -> PersistentGraphState:
    """Build graph from case assets using semantic entities and relations."""
    import re

    fixture = load_case_fixture(case_id)
    gs = PersistentGraphState()
    _TEXT_EXTS = {".md", ".txt", ".csv", ".tsv"}

    source_entity_labels: list[tuple[str, list[str]]] = []
    semantic_chunks: list[str] = []

    for asset in fixture.assets:
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

        gs.add_fragment(cas_fragment({"text": text, "artifact_id": asset.file_name}, "text/markdown"))

        # Extract entities
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
                source_fragment_id=case_id,
                entity_key=stable_entity_key(case_id, label),
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
                relation_key=stable_relation_key(case_id, predicate, src_entity.label, dst_entity.label),
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


def _find_oracle(case_id: str) -> str:
    """Locate oracle.json for a case."""
    return f"tests/fixtures/cases/{case_id}/oracle.json"


@router.post("/run")
def run_evaluation(case_id: str):
    """Run full oráculo evaluation on a case and return report."""
    oracle_path = _find_oracle(case_id)
    try:
        spec = OracleSpec.from_json(oracle_path)
    except FileNotFoundError:
        raise HTTPException(status_code=404, detail=f"Oracle fixture not found for case: {case_id}")

    try:
        graph = _build_graph(case_id)
    except FileNotFoundError:
        raise HTTPException(status_code=404, detail=f"Case fixture not found: {case_id}")

    judge_model = os.getenv("LLM_JUDGE_MODEL", "").strip() or os.getenv("LLM_MODEL", "").strip()
    judge = UnifiedJudge(model=judge_model)
    evaluator = Evaluator(judge=judge)
    report = evaluator.evaluate_all(graph, spec)
    payload = serialize_evaluation_report(report)
    payload["case_id"] = case_id
    return payload
