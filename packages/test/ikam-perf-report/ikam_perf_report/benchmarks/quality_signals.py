from __future__ import annotations

import hashlib
import json
from typing import Any


def _clamp(value: float) -> float:
    return round(min(1.0, max(0.0, value)), 3)


DEFAULT_COMMIT_THRESHOLDS = {
    "grounded_precision_floor": 0.1,
    "evidence_coverage_floor": 0.1,
    "endpoint_integrity_floor": 1.0,
    "replay_idempotency_floor": 1.0,
}


def compute_lane_metrics(
    *,
    normalized_fragments: int,
    entity_fragments: int,
    relation_fragments: int,
    graph_edges: int,
    cas_hit_rate: float,
    second_promoted: int,
    unresolved_endpoints: int = 0,
    grounded_precision_hint: float | None = None,
    evidence_coverage_hint: float | None = None,
) -> dict[str, float]:
    semantic_total = max(1, entity_fragments + relation_fragments)
    exploration_variability = _clamp(entity_fragments / max(1, normalized_fragments))
    relation_commit_yield = _clamp(relation_fragments / semantic_total)
    evidence_grounding_ratio = _clamp(
        grounded_precision_hint if grounded_precision_hint is not None else (entity_fragments / semantic_total)
    )
    evidence_coverage_ratio = _clamp(
        evidence_coverage_hint if evidence_coverage_hint is not None else (graph_edges / max(1, relation_fragments))
    )
    endpoint_integrity = _clamp(1.0 - (unresolved_endpoints / max(1, relation_fragments)))
    edge_idempotency_integrity = _clamp(1.0 if second_promoted == 0 else 0.0)
    within_case_reuse_primary = _clamp(cas_hit_rate)
    cross_case_reuse_secondary = 0.0

    return {
        "exploration_variability": exploration_variability,
        "relation_commit_yield": relation_commit_yield,
        "evidence_grounding_ratio": evidence_grounding_ratio,
        "evidence_coverage_ratio": evidence_coverage_ratio,
        "endpoint_integrity": endpoint_integrity,
        "edge_idempotency_integrity": edge_idempotency_integrity,
        "within_case_reuse_primary": within_case_reuse_primary,
        "cross_case_reuse_secondary": cross_case_reuse_secondary,
    }


def evaluate_commit_lane_gates(
    lane_metrics: dict[str, float],
    thresholds: dict[str, float] | None = None,
) -> dict[str, Any]:
    limits = dict(DEFAULT_COMMIT_THRESHOLDS)
    if thresholds:
        limits.update(thresholds)

    failures: list[str] = []
    if lane_metrics.get("evidence_grounding_ratio", 0.0) < limits["grounded_precision_floor"]:
        failures.append("grounded_precision_floor")
    if lane_metrics.get("evidence_coverage_ratio", 0.0) < limits["evidence_coverage_floor"]:
        failures.append("evidence_coverage_floor")
    if lane_metrics.get("endpoint_integrity", 0.0) < limits["endpoint_integrity_floor"]:
        failures.append("endpoint_integrity_floor")
    if lane_metrics.get("edge_idempotency_integrity", 0.0) < limits["replay_idempotency_floor"]:
        failures.append("replay_idempotency_floor")

    return {
        "passed": len(failures) == 0,
        "thresholds": limits,
        "failures": failures,
    }


def build_commit_receipt(
    *,
    case_id: str,
    mode: str,
    committed_fragment_ids: list[str],
    edge_idempotency_keys: list[str],
    unresolved_endpoints: list[str] | None = None,
) -> dict[str, Any]:
    unresolved = sorted(unresolved_endpoints or [])
    payload = {
        "case_id": case_id,
        "mode": mode,
        "target_ref": "refs/heads/main",
        "promoted_fragment_ids": sorted(committed_fragment_ids),
        "committed_fragment_ids": sorted(committed_fragment_ids),
        "edge_idempotency_keys": sorted(edge_idempotency_keys),
        "unresolved_endpoints": unresolved,
    }
    canonical = json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return {
        **payload,
        "receipt_id": hashlib.sha256(canonical.encode("utf-8")).hexdigest(),
    }


def build_query_evaluations(
    case_id: str,
    *,
    graph_nodes: int,
    graph_edges: int,
    asset_mime_types: list[str],
    normalized_fragments: int,
    entity_fragments: int,
    relation_fragments: int,
    promoted_ratio: float,
) -> list[dict[str, Any]]:
    unique_mimes = len(set(asset_mime_types))
    total_semantic = max(1, entity_fragments + relation_fragments)
    edge_density = graph_edges / max(1, graph_nodes)
    storage_gain = 1.0 - (graph_nodes / max(1, normalized_fragments + total_semantic))

    business_coverage = _clamp(unique_mimes / 12.0)
    business_precision = _clamp(entity_fragments / total_semantic)

    storage_coverage = _clamp(storage_gain)
    storage_precision = _clamp(promoted_ratio)

    reliability_coverage = _clamp(edge_density / 0.8)
    reliability_precision = _clamp(relation_fragments / max(1, normalized_fragments))

    return [
        {
            "query_id": f"{case_id}:business-identity",
            "oracle": {"coverage": business_coverage, "grounded_precision": business_precision},
            "review": None,
        },
        {
            "query_id": f"{case_id}:storage-gains",
            "oracle": {"coverage": storage_coverage, "grounded_precision": storage_precision},
            "review": None,
        },
        {
            "query_id": f"{case_id}:reliability",
            "oracle": {"coverage": reliability_coverage, "grounded_precision": reliability_precision},
            "review": None,
        },
    ]
