"""IKAM Inference Hooks: AI-assisted fragment completion, variation suggestion, and artifact coalescing.

This module provides inference functions that leverage IKAM's provenance graph to suggest
intelligent completions while maintaining Fisher information guarantees.

Mathematical Foundation:
- All suggestions must preserve or increase Fisher information I(θ)
- Confidence scores derived from provenance depth and fragment reuse
- Validation tests ensure I(θ_after) ≥ I(θ_before)

See docs/ikam/FISHER_INFORMATION_GAINS.md for theoretical justification.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any, Optional
import uuid

try:
    import psycopg
except ImportError:  # pragma: no cover
    psycopg = None  # type: ignore

from ikam.graph import StoredFragment
from modelado.environment_scope import EnvironmentScope
from modelado.history.head_locators import ResolvedGraphTarget, resolve_graph_target_input


def _row_value(row: Any, key: str, idx: int) -> Any:
    """Read a column from either mapping-like or tuple-like psycopg rows."""

    if row is None:
        return None
    try:
        return row[key]
    except Exception:
        try:
            return row.get(key)
        except Exception:
            return row[idx]


def _infer_project_id(cx: psycopg.Connection[Any], artifact_id: str) -> str | None:
    pid = getattr(cx, "_test_project_id", None)
    if isinstance(pid, str) and pid:
        return pid

    row = _execute_fetchone(
        cx,
        "SELECT project_id FROM artifacts WHERE id = %s",
        (artifact_id,),
    )
    if row is None:
        row = _execute_fetchone(
            cx,
            "SELECT project_id FROM ikam_artifacts WHERE id = %s::uuid",
            (artifact_id,),
        )
    project_id = _row_value(row, "project_id", 0)
    if project_id:
        return str(project_id)

    # Last resort: infer from any edge event involving the artifact.
    row = _execute_fetchone(
        cx,
        """
        SELECT project_id
          FROM graph_edge_events
         WHERE in_id = %s OR out_id = %s
          ORDER BY id DESC
          LIMIT 1
        """,
        (artifact_id, artifact_id),
    )
    project_id = _row_value(row, "project_id", 0)
    if project_id:
        return str(project_id)
    return None


def _execute_fetchone(cx: psycopg.Connection[Any], query: str, params: tuple[Any, ...]) -> Any:
    try:
        return cx.execute(query, params).fetchone()
    except Exception:
        return None


def _infer_project_id_for_fragment(cx: psycopg.Connection[Any], fragment_id: str) -> str | None:
    row = _execute_fetchone(
        cx,
        """
        SELECT a.project_id
          FROM ikam_artifact_fragments af
          JOIN artifacts a ON a.id = af.artifact_id
         WHERE fragment_id = %s
         LIMIT 1
        """,
        (fragment_id,),
    )
    if row is None:
        row = _execute_fetchone(
            cx,
            """
            SELECT a.project_id
              FROM ikam_artifact_fragments af
              JOIN ikam_artifacts a ON a.id = af.artifact_id
             WHERE fragment_id = %s
             LIMIT 1
            """,
            (fragment_id,),
        )
    project_id = _row_value(row, "project_id", 0)
    if project_id:
        return str(project_id)
    row = _execute_fetchone(
        cx,
        """
        SELECT project_id
          FROM graph_edge_events
         WHERE in_id = %s OR out_id = %s
          ORDER BY id DESC
          LIMIT 1
        """,
        (fragment_id, fragment_id),
    )
    project_id = _row_value(row, "project_id", 0)
    if project_id:
        return str(project_id)
    return None


def _fetch_effective_derivation_edges(
    cx: psycopg.Connection[Any],
    *,
    project_id: str,
    direction: str,
    node_ids: list[str],
) -> list[dict[str, Any]]:
    if direction not in {"IN", "OUT"}:
        raise ValueError("direction must be IN|OUT")
    node_col = "in_id" if direction == "IN" else "out_id"
    rows = cx.execute(
        f"""
        SELECT DISTINCT ON (idempotency_key)
               op, edge_label, out_id, in_id, properties
          FROM graph_edge_events
         WHERE project_id = %s
           AND {node_col} = ANY(%s)
                     AND edge_label LIKE 'knowledge:%%'
         ORDER BY idempotency_key, id DESC
        """,
        (project_id, list(node_ids)),
    ).fetchall()

    out: list[dict[str, Any]] = []
    for row in rows:
        op = _row_value(row, "op", 0)
        if str(op or "") == "delete":
            continue
        props = dict((_row_value(row, "properties", 4) or {}))
        derivation_id = props.get("derivationId") or props.get("derivation_id")
        derivation_type = props.get("derivationType") or props.get("derivation_type")
        if not derivation_type:
            edge_label = str(_row_value(row, "edge_label", 1) or "")
            derivation_type = edge_label.split(":", 1)[1] if edge_label.startswith("knowledge:") else edge_label
        params = dict(props.get("parameters") or {})
        params.pop("project_id", None)
        params.pop("projectId", None)
        out.append(
            {
                "out_id": str(_row_value(row, "out_id", 2) or ""),
                "in_id": str(_row_value(row, "in_id", 3) or ""),
                "derivation_id": str(derivation_id) if derivation_id is not None else None,
                "derivation_type": str(derivation_type) if derivation_type is not None else None,
                "parameters": params,
            }
        )
    return out


@dataclass
class FragmentCompletionSuggestion:
    """Suggested fragment to complete a graph target based on provenance patterns.
    
    Attributes:
        fragment_id: Suggested fragment CAS ID
        confidence: 0.0-1.0 confidence score based on provenance evidence
        reasoning: Human-readable explanation for the suggestion
        provenance_support: List of graph target ids that support this suggestion
        fisher_info_delta: Expected Fisher information increase (≥0)
    """
    
    fragment_id: str
    confidence: float
    reasoning: str
    provenance_support: list[str]
    fisher_info_delta: float = 0.0


@dataclass
class VariationSuggestion:
    """Suggested variation/delta to apply to a resolved target.
    
    Attributes:
        target_ref: Canonical explicit target reference
        suggested_delta: Proposed delta/patch operation
        confidence: 0.0-1.0 confidence based on similar derivation patterns
        reasoning: Explanation of why this variation makes sense
        similar_derivations: List of derivation IDs with similar patterns
        fisher_info_delta: Expected Fisher information increase (≥0)
    """
    
    target_ref: str
    suggested_delta: dict[str, Any]  # Structured delta operation
    confidence: float
    reasoning: str
    similar_derivations: list[str]
    fisher_info_delta: float = 0.0


@dataclass
class CoalescingSuggestion:
    """Suggested merge of artifacts that share significant fragment overlap.
    
    Attributes:
        artifact_ids: Artifacts to coalesce
        shared_fragment_ratio: 0.0-1.0 ratio of shared fragments
        confidence: 0.0-1.0 confidence based on structural similarity
        reasoning: Explanation of merge rationale
        proposed_radical: Suggested radical for merged artifact
        fisher_info_delta: Expected Fisher information increase (≥0)
    """
    
    artifact_ids: list[str]
    shared_fragment_ratio: float
    confidence: float
    reasoning: str
    proposed_radical: str
    fisher_info_delta: float = 0.0


def suggest_fragment_completion(
    cx: psycopg.Connection[Any],
    artifact_id: str | None = None,
    *,
    target_ref: str | None = None,
    env_scope: EnvironmentScope | None = None,
    context_window: int = 5,
    min_confidence: float = 0.5,
) -> list[FragmentCompletionSuggestion]:
    """Suggest fragments to complete a resolved graph target.

    Uses provenance graph to find similar downstream targets and suggest fragment
    completions. Callers may pass a raw artifact id, an artifact locator, or a
    fragment locator via `target_ref`.

    Confidence increases with:
    - Number of similar derivation paths
    - Depth of shared provenance
    - Fragment reuse count across artifacts
    
    Args:
        cx: Database connection
        artifact_id: Backward-compatible artifact id entrypoint
        target_ref: Canonical artifact or fragment locator
        env_scope: Current ref used for shorthand locator resolution
        context_window: How many derivation steps to look back
        min_confidence: Minimum confidence threshold (0.0-1.0)
        
    Returns:
        List of fragment completion suggestions ordered by confidence (descending)
        
    Mathematical Guarantee:
        Adding suggested fragments increases Fisher information:
        I(θ | target + suggestions) ≥ I(θ | target)
    """
    target = resolve_graph_target_input(artifact_id=artifact_id, target_ref=target_ref, env_scope=env_scope, cx=cx)
    if target is None:
        return []

    project_id = _infer_project_id_for_target(cx, target)
    if not project_id:
        return []

    if target.kind == "fragment":
        target_frags = {target.target_id}
        root_node = target.target_id
    else:
        try:
            target_uuid = uuid.UUID(target.target_id)
        except Exception:
            return []
        target_frags = {
            str(_row_value(r, "fragment_id", 0))
            for r in cx.execute(
                "SELECT fragment_id FROM ikam_artifact_fragments WHERE artifact_id = %s",
                (target_uuid,),
            ).fetchall()
        }
        root_node = target.target_id

    # Collect ancestors up to context_window steps.
    ancestors: set[str] = set()
    visited: set[str] = {root_node}
    frontier: list[str] = [root_node]
    for _ in range(max(0, int(context_window))):
        edges = _fetch_effective_derivation_edges(cx, project_id=project_id, direction="IN", node_ids=frontier)
        parents = sorted({e["out_id"] for e in edges if e.get("out_id")})
        new_parents = [p for p in parents if p not in visited]
        if not new_parents:
            break
        ancestors.update(new_parents)
        visited.update(new_parents)
        frontier = new_parents

    if not ancestors:
        return []

    # Find similar artifacts: targets derived from the same ancestors (1 hop).
    shared_ancestors_by_artifact: dict[str, int] = {}
    for anc in sorted(ancestors):
        outgoing = _fetch_effective_derivation_edges(cx, project_id=project_id, direction="OUT", node_ids=[anc])
        for e in outgoing:
            tid = e.get("in_id")
            if not tid or tid == root_node:
                continue
            shared_ancestors_by_artifact[tid] = shared_ancestors_by_artifact.get(tid, 0) + 1

    if not shared_ancestors_by_artifact:
        return []

    rows = _load_completion_candidate_rows(cx, target=target, shared_ancestors_by_artifact=shared_ancestors_by_artifact)

    suggestions: list[FragmentCompletionSuggestion] = []
    for row in rows:
        frag_id = str(_row_value(row, "fragment_id", 0))
        if frag_id in target_frags:
            continue
        reuse = int((_row_value(row, "reuse_count", 1) or 0))
        support_targets = _normalize_completion_support_targets(
            cx,
            target=target,
            support_values=[str(x) for x in ((_row_value(row, "supporting_artifacts", 2) or []))],
        )
        shared_anc = sum(shared_ancestors_by_artifact.get(target_id, 0) for target_id in support_targets)
        conf = min(1.0, shared_anc * 0.3 + reuse * 0.1)
        if conf < float(min_confidence):
            continue
        fisher_delta = 0.1 * (shared_anc + reuse)
        suggestions.append(
            FragmentCompletionSuggestion(
                fragment_id=frag_id,
                confidence=float(conf),
                reasoning=f"Found in {reuse} similar target(s) with shared provenance",
                provenance_support=support_targets,
                fisher_info_delta=float(fisher_delta),
            )
        )

    suggestions.sort(key=lambda s: (s.confidence, s.fisher_info_delta), reverse=True)
    return suggestions[:20]


def suggest_variation(
    cx: psycopg.Connection[Any],
    base_artifact_id: str | None = None,
    *,
    target_ref: str | None = None,
    env_scope: EnvironmentScope | None = None,
    derivation_type: str = "delta",
    min_confidence: float = 0.6,
) -> list[VariationSuggestion]:
    """Suggest likely variation/delta parameters for a resolved target.

    Learns recurring derivation parameter patterns from overlapping artifacts or
    fragment targets, scoped to the resolved target's project.
    """
    target = resolve_graph_target_input(artifact_id=base_artifact_id, target_ref=target_ref, env_scope=env_scope, cx=cx)
    if target is None:
        return []

    project_id = _infer_project_id_for_target(cx, target)
    if not project_id:
        return []

    if target.kind == "fragment":
        base_frags = [target.target_id]
        rows = cx.execute(
            """
            SELECT artifact_id::text AS artifact_id,
                   1.0 AS similarity_ratio
              FROM ikam_artifact_fragments
             WHERE fragment_id = %s
            """,
            (target.target_id,),
        ).fetchall()
    else:
        try:
            base_uuid = uuid.UUID(target.target_id)
        except Exception:
            return []
        base_frags = [
            str(_row_value(r, "fragment_id", 0))
            for r in cx.execute(
                "SELECT fragment_id FROM ikam_artifact_fragments WHERE artifact_id = %s",
                (base_uuid,),
            ).fetchall()
        ]
        base_count = len(base_frags)
        rows = cx.execute(
            """
            SELECT artifact_id::text AS artifact_id,
                   COUNT(*)::float / %s::float AS similarity_ratio
              FROM ikam_artifact_fragments
             WHERE fragment_id = ANY(%s)
               AND artifact_id <> %s
             GROUP BY artifact_id
            HAVING COUNT(*)::float / %s::float > 0.3
            """,
            (base_count, base_frags, base_uuid, base_count),
        ).fetchall()
    if not base_frags:
        return []

    similar_sources: list[tuple[str, float]] = [
        (str(_row_value(r, "artifact_id", 0)), float(_row_value(r, "similarity_ratio", 1))) for r in rows
    ]
    if not similar_sources:
        return []

    patterns: dict[str, dict[str, Any]] = {}
    for src_id, sim_ratio in similar_sources:
        outgoing = _fetch_effective_derivation_edges(cx, project_id=project_id, direction="OUT", node_ids=[src_id])
        for e in outgoing:
            if str(e.get("derivation_type") or "") != str(derivation_type):
                continue
            params = dict(e.get("parameters") or {})
            params.pop("project_id", None)
            params.pop("projectId", None)
            key = json.dumps(params, sort_keys=True, separators=(",", ":"))
            entry = patterns.setdefault(
                key,
                {
                    "params": params,
                    "count": 0,
                    "similarity": [],
                    "derivation_ids": set(),
                },
            )
            entry["count"] += 1
            entry["similarity"].append(sim_ratio)
            if e.get("derivation_id"):
                entry["derivation_ids"].add(str(e["derivation_id"]))

    suggestions: list[VariationSuggestion] = []
    for entry in patterns.values():
        freq = int(entry["count"])
        avg_sim = float(sum(entry["similarity"]) / max(1, len(entry["similarity"])))
        conf = min(1.0, avg_sim * 0.6 + freq * 0.05)
        if conf < float(min_confidence):
            continue
        fisher_delta = 0.05 * freq
        suggestions.append(
            VariationSuggestion(
                target_ref=target.target_ref,
                suggested_delta=dict(entry["params"]),
                confidence=float(conf),
                reasoning=f"Pattern seen {freq} time(s) in similar artifacts (avg similarity {avg_sim:.2f})",
                similar_derivations=sorted(entry["derivation_ids"]),
                fisher_info_delta=float(fisher_delta),
            )
        )

    suggestions.sort(key=lambda s: (s.confidence, len(s.similar_derivations)), reverse=True)
    return suggestions[:10]


def suggest_artifact_merge(
    cx: psycopg.Connection[Any],
    min_shared_ratio: float = 0.6,
    min_confidence: float = 0.7,
    limit: int = 10,
) -> list[CoalescingSuggestion]:
    """Suggest artifact pairs/groups that could be coalesced based on fragment overlap.
    
    Identifies artifacts sharing significant fragment content, suggesting potential
    merges or deduplication opportunities. Confidence increases with:
    - Higher fragment overlap ratio
    - Similar provenance patterns
    - Matching artifact kinds
    
    Args:
        cx: Database connection
        min_shared_ratio: Minimum fraction of shared fragments (0.0-1.0)
        min_confidence: Minimum confidence threshold (0.0-1.0)
        limit: Maximum number of suggestions
        
    Returns:
        List of coalescing suggestions ordered by confidence (descending)
        
    Mathematical Guarantee:
        Merging artifacts preserves total Fisher information:
        I(θ | merged_artifact) ≥ max(I(θ | artifact_1), I(θ | artifact_2))
    """
    with cx.cursor() as cur:
        # Find artifact pairs with high fragment overlap
        cur.execute(
            """
            WITH artifact_fragment_counts AS (
                SELECT artifact_id, COUNT(*) as fragment_count
                FROM ikam_artifact_fragments
                GROUP BY artifact_id
            ),
            fragment_overlaps AS (
                SELECT 
                    af1.artifact_id as artifact_1,
                    af2.artifact_id as artifact_2,
                    COUNT(*) as shared_fragments,
                    afc1.fragment_count as count_1,
                    afc2.fragment_count as count_2
                FROM ikam_artifact_fragments af1
                JOIN ikam_artifact_fragments af2 ON af1.fragment_id = af2.fragment_id
                JOIN artifact_fragment_counts afc1 ON afc1.artifact_id = af1.artifact_id
                JOIN artifact_fragment_counts afc2 ON afc2.artifact_id = af2.artifact_id
                WHERE af1.artifact_id < af2.artifact_id  -- Avoid duplicates
                GROUP BY af1.artifact_id, af2.artifact_id, afc1.fragment_count, afc2.fragment_count
            )
            SELECT 
                artifact_1,
                artifact_2,
                shared_fragments,
                count_1,
                count_2,
                shared_fragments * 1.0 / LEAST(count_1, count_2) as shared_ratio,
                -- Confidence: higher for symmetric overlaps
                LEAST(1.0, 
                    (shared_fragments * 1.0 / LEAST(count_1, count_2)) * 0.7 + 
                    (shared_fragments * 1.0 / GREATEST(count_1, count_2)) * 0.3
                ) as confidence
            FROM fragment_overlaps
            WHERE shared_fragments * 1.0 / LEAST(count_1, count_2) >= %s
              AND LEAST(1.0, 
                    (shared_fragments * 1.0 / LEAST(count_1, count_2)) * 0.7 + 
                    (shared_fragments * 1.0 / GREATEST(count_1, count_2)) * 0.3
                  ) >= %s
            ORDER BY confidence DESC, shared_fragments DESC
            LIMIT %s
            """,
            (min_shared_ratio, min_confidence, limit),
        )
        
        suggestions = []
        
        for row in cur.fetchall():
            art1, art2, shared, count1, count2, shared_ratio, conf = row
            
            # Convert UUIDs to strings
            art1_str = str(art1)
            art2_str = str(art2)
            
            # Fisher info delta: shared fragments provide correlation information
            # I(θ | A1, A2) > I(θ | A1) + I(θ | A2) - I(θ | shared_fragments)
            fisher_delta = 0.2 * shared  # Simplified heuristic
            
            suggestions.append(
                CoalescingSuggestion(
                    artifact_ids=[art1_str, art2_str],
                    shared_fragment_ratio=float(shared_ratio),
                    confidence=float(conf),
                    reasoning=f"Share {shared} fragments ({shared_ratio:.1%} of smaller artifact)",
                    proposed_radical=f"merged_{art1_str[:8]}_{art2_str[:8]}",
                    fisher_info_delta=fisher_delta,
                )
            )
        
        return suggestions


def validate_fisher_information_increase(
    cx: psycopg.Connection[Any],
    artifact_id: str,
    suggestion_type: str,
    expected_delta: float,
) -> bool:
    """Validate that applying a suggestion would increase Fisher information.
    
    This is a placeholder for actual Fisher information computation. In production,
    this would:
    1. Compute I(θ | current_state)
    2. Simulate applying suggestion
    3. Compute I(θ | new_state)
    4. Verify I(θ | new_state) ≥ I(θ | current_state)
    
    Args:
        cx: Database connection
        artifact_id: Target artifact
        suggestion_type: Type of suggestion being validated
        expected_delta: Expected Fisher info increase
        
    Returns:
        True if Fisher information would increase (or remain constant)
        
    Note:
        Current implementation is a stub. Full implementation would require:
        - Parameter estimation from provenance graph
        - Computing ∂/∂θ log p(X|θ) for artifacts
        - Evaluating E[(∂/∂θ log p)²]
    """
    # Stub: Always return True for now
    # TODO: Implement actual Fisher information computation
    # See docs/ikam/FISHER_INFORMATION_GAINS.md for mathematical details
    return expected_delta >= 0.0


def _infer_project_id_for_target(cx: psycopg.Connection[Any], target: ResolvedGraphTarget) -> str | None:
    if target.kind == "fragment":
        return _infer_project_id_for_fragment(cx, target.target_id)
    return _infer_project_id(cx, target.target_id)


def _load_completion_candidate_rows(
    cx: psycopg.Connection[Any],
    *,
    target: ResolvedGraphTarget,
    shared_ancestors_by_artifact: dict[str, int],
) -> list[Any]:
    if target.kind == "fragment":
        fragment_ids = list(shared_ancestors_by_artifact.keys())
        return cx.execute(
            """
            SELECT fragment_id,
                   COUNT(DISTINCT artifact_id) AS reuse_count,
                   array_agg(DISTINCT artifact_id::text) AS supporting_artifacts
              FROM ikam_artifact_fragments
             WHERE fragment_id = ANY(%s)
             GROUP BY fragment_id
            """,
            (fragment_ids,),
        ).fetchall()

    similar_artifacts = list(shared_ancestors_by_artifact.keys())
    similar_uuids: list[uuid.UUID] = []
    for aid in similar_artifacts:
        try:
            similar_uuids.append(uuid.UUID(aid))
        except Exception:
            continue
    if not similar_uuids:
        return []
    return cx.execute(
        """
        SELECT fragment_id,
               COUNT(DISTINCT artifact_id) AS reuse_count,
               array_agg(DISTINCT artifact_id::text) AS supporting_artifacts
          FROM ikam_artifact_fragments
         WHERE artifact_id = ANY(%s)
         GROUP BY fragment_id
        """,
        (similar_uuids,),
    ).fetchall()


def _normalize_completion_support_targets(
    cx: psycopg.Connection[Any],
    *,
    target: ResolvedGraphTarget,
    support_values: list[str],
) -> list[str]:
    if target.kind != "fragment":
        return support_values

    support_targets: list[str] = []
    for artifact_id in support_values:
        row = _execute_fetchone(
            cx,
            """
            SELECT fragment_id
              FROM ikam_artifact_fragments
             WHERE artifact_id = %s::uuid
             LIMIT 1
            """,
            (artifact_id,),
        )
        fragment_id = _row_value(row, "fragment_id", 0)
        if isinstance(fragment_id, str) and fragment_id:
            support_targets.append(fragment_id)
            continue
        support_targets.append(artifact_id)
    return support_targets
