from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any, Iterable, Mapping, Sequence
import hashlib
import json

try:
    import psycopg
except Exception:  # pragma: no cover - optional in pure-logic tests
    psycopg = None  # type: ignore

from ikam.fragments import Fragment as V3Fragment, Relation, RELATION_MIME
from ikam.graph import StoredFragment as StorageFragment, _cas_hex

from modelado.graph_edge_event_folding import delete_matching_subtree_edges, is_subtree_graph_delta_delete
from modelado.graph_edge_event_log import GraphEdgeEvent, compute_edge_identity_key
from modelado.knowledge_edge_events import KnowledgeEdgeEventInput, build_knowledge_edge_events


def _stable_json(payload: Mapping[str, Any]) -> str:
    return json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def _hash_hex(payload: Mapping[str, Any]) -> str:
    blob = _stable_json(payload).encode("utf-8")
    return hashlib.blake2b(blob, digest_size=16).hexdigest()


def _unique_sorted(values: Iterable[str]) -> list[str]:
    out: list[str] = []
    seen: set[str] = set()
    for raw in values:
        if not isinstance(raw, str):
            continue
        val = raw.strip()
        if not val or val in seen:
            continue
        seen.add(val)
        out.append(val)
    out.sort()
    return out


# ---------------------------------------------------------------------------
# O1: Semantic kind inference (mandatory, deterministic selection)
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class KindCandidate:
    kind: str
    confidence: float
    evidence: Mapping[str, Any]


@dataclass(frozen=True)
class KindInferenceResult:
    kind: str
    confidence: float
    evidence: dict[str, Any]
    evidence_hash: str
    candidates: list[KindCandidate]


def infer_artifact_kind(candidates: Sequence[KindCandidate]) -> KindInferenceResult:
    """Select a semantic artifact kind deterministically.

    This operator does not inspect bytes or hardcode type switches; it selects
    from externally-provided semantic candidates and preserves evidence.
    """
    if not candidates:
        raise ValueError("O1 requires at least one semantic kind candidate")

    normalized: list[KindCandidate] = []
    for c in candidates:
        kind = str(c.kind).strip()
        if not kind:
            continue
        confidence = float(c.confidence)
        if not (0.0 <= confidence <= 1.0):
            raise ValueError(f"O1 candidate confidence out of range: {confidence}")
        evidence = dict(c.evidence or {})
        normalized.append(KindCandidate(kind=kind, confidence=confidence, evidence=evidence))

    if not normalized:
        raise ValueError("O1 candidates must include at least one non-empty kind")

    chosen = sorted(normalized, key=lambda c: (-c.confidence, c.kind))[0]
    evidence_hash = _hash_hex({"kind": chosen.kind, "evidence": chosen.evidence})
    return KindInferenceResult(
        kind=chosen.kind,
        confidence=chosen.confidence,
        evidence=dict(chosen.evidence),
        evidence_hash=evidence_hash,
        candidates=list(normalized),
    )


# ---------------------------------------------------------------------------
# O2: Decompose (domain fragment tree)
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class DecompositionResult:
    artifact_id: str
    fragments: list[V3Fragment]
    strategy: str
    created_at: datetime


def _validate_fragment_tree(artifact_id: str, fragments: Sequence[V3Fragment]) -> None:
    if not fragments:
        raise ValueError("O2 requires at least one domain fragment")
    for f in fragments:
        if not f.cas_id and f.value is None:
            raise ValueError("O2 fragments must satisfy V3 invariant F1")


def decompose_bytes(
    *,
    artifact_id: str,
    payload: bytes,
    mime_type: str,
    fragments: Sequence[V3Fragment] | None = None,
    strategy: str = "opaque_binary",
    metadata: Mapping[str, Any] | None = None,
) -> DecompositionResult:
    """Decompose bytes into a rooted domain fragment tree.

    Mathematical guarantee preserved: reconstruct(decompose(A)) = A.
    """
    if fragments:
        _validate_fragment_tree(artifact_id, fragments)
        return DecompositionResult(
            artifact_id=artifact_id,
            fragments=list(fragments),
            strategy=strategy,
            created_at=datetime.now(timezone.utc),
        )

    if not payload:
        raise ValueError("O2 requires non-empty payload bytes")

    storage_fragment = StorageFragment.from_bytes(payload, mime_type)
    root = V3Fragment(
        cas_id=storage_fragment.id,
        value=payload,
        mime_type=mime_type,
    )

    return DecompositionResult(
        artifact_id=artifact_id,
        fragments=[root],
        strategy=strategy,
        created_at=datetime.now(timezone.utc),
    )


# ---------------------------------------------------------------------------
# O3: Canonical serialize + CAS store
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class CasStoreResult:
    domain_id_to_cas_id: dict[str, str]
    cas_ids: list[str]


def canonicalize_and_store_fragments(
    *,
    fragments: Sequence[V3Fragment],
    cx: "psycopg.Connection[Any] | None" = None,
) -> CasStoreResult:
    """Canonicalize fragments into CAS ids and optionally persist.

    Deterministic CAS ids are derived from canonical domain serialization.
    """
    if not fragments:
        raise ValueError("O3 requires at least one domain fragment")

    domain_id_to_cas_id: dict[str, str] = {}
    for fragment in fragments:
        cas_id = fragment.cas_id
        if not cas_id:
            if isinstance(fragment.value, (bytes, bytearray)):
                cas_id = _cas_hex(bytes(fragment.value))
            else:
                cas_id = _cas_hex(json.dumps(fragment.value, sort_keys=True).encode("utf-8"))
        domain_id_to_cas_id[cas_id] = cas_id

    if cx is not None:
        from modelado.ikam_graph_repository import insert_domain_fragments

        insert_domain_fragments(cx, list(fragments))

    cas_ids = sorted(domain_id_to_cas_id.values())
    return CasStoreResult(domain_id_to_cas_id=domain_id_to_cas_id, cas_ids=cas_ids)


# ---------------------------------------------------------------------------
# Realization records (Contract D1, D2, D3)
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class RealizationRecord:
    """Immutable record of an effectful operation's execution.

    Contract D2: persists full replay metadata (operator_spec, inputs,
    policy, exogenous_fingerprint, outcome_fid).
    Contract D1: outcome_fid is a canonical CAS identity.
    Contract D3: deterministic record_id from content hash.
    """

    record_id: str
    operator_spec: dict[str, Any]
    inputs: dict[str, Any]
    policy: dict[str, Any]
    exogenous_fingerprint: str
    outcome_fid: str
    created_at: datetime


def build_realization_record(
    *,
    operator_spec: Mapping[str, Any],
    inputs: Mapping[str, Any],
    policy: Mapping[str, Any],
    exogenous: Mapping[str, Any],
    outcome_fragments: Sequence[V3Fragment],
) -> RealizationRecord:
    """Build a deterministic realization record for an effectful operation.

    Contract D1: outcome_fid is derived via canonical CAS identity.
    Contract D2: all required replay fields are populated.
    Contract D3: record_id is deterministic from content, enabling replay verification.
    """
    if not outcome_fragments:
        raise ValueError("Realization requires at least one outcome fragment")

    # D1: canonical CAS identity for outcome
    first = outcome_fragments[0]
    if first.cas_id:
        outcome_fid = first.cas_id
    elif isinstance(first.value, (bytes, bytearray)):
        outcome_fid = _cas_hex(bytes(first.value))
    else:
        outcome_fid = _cas_hex(json.dumps(first.value, sort_keys=True).encode("utf-8"))

    # Exogenous fingerprint: deterministic hash of exogenous context
    exogenous_fingerprint = _hash_hex(dict(exogenous))

    # D3: deterministic record identity from all replay-relevant fields
    record_id = _hash_hex({
        "operator_spec": dict(operator_spec),
        "inputs": dict(inputs),
        "policy": dict(policy),
        "exogenous_fingerprint": exogenous_fingerprint,
        "outcome_fid": outcome_fid,
    })

    return RealizationRecord(
        record_id=record_id,
        operator_spec=dict(operator_spec),
        inputs=dict(inputs),
        policy=dict(policy),
        exogenous_fingerprint=exogenous_fingerprint,
        outcome_fid=outcome_fid,
        created_at=datetime.now(timezone.utc),
    )


# ---------------------------------------------------------------------------
# O5: Context selection / grounding retrieval
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class TraversalPolicy:
    max_depth: int = 1
    edge_label_prefixes: tuple[str, ...] = ("derivation:", "knowledge:")
    include_upstream: bool = True
    include_downstream: bool = False
    max_nodes: int = 2_000


@dataclass(frozen=True)
class ContextConstraints:
    must_link: list[tuple[str, str]] = field(default_factory=list)
    cannot_link: list[tuple[str, str]] = field(default_factory=list)
    capacity: dict[str, int] = field(default_factory=dict)


@dataclass(frozen=True)
class ContextEdge:
    edge_label: str
    out_id: str
    in_id: str
    properties: dict[str, Any]
    edge_key: str


@dataclass(frozen=True)
class ContextExplanation:
    target_id: str
    path: list[str]
    edge_keys: list[str]


@dataclass(frozen=True)
class ContextBundle:
    bundle_id: str
    project_id: str
    seed_artifact_ids: list[str]
    seed_fragment_ids: list[str]
    artifact_ids: list[str]
    fragment_ids: list[str]
    edges: list[ContextEdge]
    explanations: list[ContextExplanation]
    constraints: ContextConstraints
    policy: TraversalPolicy
    created_at: datetime


def _fold_effective_edges(events: Sequence[GraphEdgeEvent]) -> list[ContextEdge]:
    effective: dict[str, ContextEdge] = {}
    for e in events:
        key = compute_edge_identity_key(
            edge_label=e.edge_label,
            out_id=e.out_id,
            in_id=e.in_id,
            properties=e.properties,
        )
        if e.op == "delete":
            if is_subtree_graph_delta_delete(e):
                delete_matching_subtree_edges(effective, e)
                continue
            effective.pop(key, None)
            continue
        effective[key] = ContextEdge(
            edge_label=e.edge_label,
            out_id=e.out_id,
            in_id=e.in_id,
            properties=dict(e.properties or {}),
            edge_key=key,
        )
    return sorted(
        effective.values(),
        key=lambda e: (e.edge_label, e.out_id, e.in_id, e.edge_key),
    )


def build_context_bundle(
    *,
    project_id: str,
    seed_artifact_ids: Sequence[str] | None = None,
    seed_fragment_ids: Sequence[str] | None = None,
    policy: TraversalPolicy | None = None,
    constraints: ContextConstraints | None = None,
    edge_events: Sequence[GraphEdgeEvent] | None = None,
    cx: "psycopg.Connection[Any] | None" = None,
) -> ContextBundle:
    """Build a deterministic context bundle with explicit traversal policy."""
    seeds_art = _unique_sorted(seed_artifact_ids or [])
    seeds_frag = _unique_sorted(seed_fragment_ids or [])

    if not seeds_art and not seeds_frag:
        raise ValueError("O5 requires at least one seed artifact or fragment id")

    policy = policy or TraversalPolicy()
    constraints = constraints or ContextConstraints()

    if edge_events is None:
        if cx is None:
            raise ValueError("O5 requires edge_events or a database connection")
        edge_events = _load_edge_events(cx, project_id=project_id, policy=policy)

    edges = _fold_effective_edges(edge_events)
    if policy.edge_label_prefixes:
        edges = [
            e
            for e in edges
            if any(str(e.edge_label).startswith(prefix) for prefix in policy.edge_label_prefixes)
        ]

    artifact_ids, explanations = _traverse_artifacts(
        seeds_art,
        edges=edges,
        policy=policy,
    )

    fragment_ids = list(seeds_frag)
    if cx is not None and artifact_ids:
        fragment_ids.extend(_fetch_fragment_ids(cx, artifact_ids))
    fragment_ids = _unique_sorted(fragment_ids)

    bundle_id = _hash_hex(
        {
            "project_id": project_id,
            "seed_artifact_ids": seeds_art,
            "seed_fragment_ids": seeds_frag,
            "artifact_ids": artifact_ids,
            "fragment_ids": fragment_ids,
            "edge_keys": [e.edge_key for e in edges],
            "policy": policy.__dict__,
            "constraints": constraints.__dict__,
        }
    )

    return ContextBundle(
        bundle_id=bundle_id,
        project_id=project_id,
        seed_artifact_ids=seeds_art,
        seed_fragment_ids=seeds_frag,
        artifact_ids=artifact_ids,
        fragment_ids=fragment_ids,
        edges=edges,
        explanations=explanations,
        constraints=constraints,
        policy=policy,
        created_at=datetime.now(timezone.utc),
    )


def _load_edge_events(
    cx: "psycopg.Connection[Any]",
    *,
    project_id: str,
    policy: TraversalPolicy,
) -> list[GraphEdgeEvent]:
    conditions = []
    params: list[Any] = [project_id]
    for prefix in policy.edge_label_prefixes:
        conditions.append("edge_label LIKE %s")
        params.append(f"{prefix}%")

    where = " OR ".join(conditions) if conditions else "TRUE"

    rows = cx.execute(
        f"""
        SELECT id, project_id, op, edge_label, out_id, in_id, properties, t, idempotency_key
          FROM graph_edge_events
         WHERE project_id = %s AND ({where})
         ORDER BY id ASC
        """,
        tuple(params),
    ).fetchall()

    events: list[GraphEdgeEvent] = []
    for row in rows:
        events.append(
            GraphEdgeEvent(
                id=int(row["id"]),
                project_id=str(row["project_id"]),
                op=str(row["op"]),
                edge_label=str(row["edge_label"]),
                out_id=str(row["out_id"]),
                in_id=str(row["in_id"]),
                properties=dict(row.get("properties") or {}),
                t=int(row.get("t") or 0),
                idempotency_key=row.get("idempotency_key"),
            )
        )
    return events


def _traverse_artifacts(
    seeds_art: Sequence[str],
    *,
    edges: Sequence[ContextEdge],
    policy: TraversalPolicy,
) -> tuple[list[str], list[ContextExplanation]]:
    visited: set[str] = set(seeds_art)
    parents: dict[str, tuple[str, str]] = {}
    frontier: list[tuple[str, int]] = [(s, 0) for s in seeds_art]

    edge_by_node: dict[str, ContextEdge] = {}
    adjacency_in: dict[str, list[ContextEdge]] = {}
    adjacency_out: dict[str, list[ContextEdge]] = {}
    for e in edges:
        adjacency_in.setdefault(e.in_id, []).append(e)
        adjacency_out.setdefault(e.out_id, []).append(e)

    for lst in adjacency_in.values():
        lst.sort(key=lambda e: (e.edge_label, e.out_id, e.in_id, e.edge_key))
    for lst in adjacency_out.values():
        lst.sort(key=lambda e: (e.edge_label, e.out_id, e.in_id, e.edge_key))

    while frontier:
        node, depth = frontier.pop(0)
        if depth >= policy.max_depth:
            continue

        next_edges: list[ContextEdge] = []
        if policy.include_upstream:
            next_edges.extend(adjacency_in.get(node, []))
        if policy.include_downstream:
            next_edges.extend(adjacency_out.get(node, []))

        for e in next_edges:
            neighbor = e.out_id if e.in_id == node else e.in_id
            if neighbor in visited:
                continue
            if len(visited) >= policy.max_nodes:
                return sorted(visited), _build_explanations(seeds_art, parents, edge_by_node)
            visited.add(neighbor)
            parents[neighbor] = (node, e.edge_key)
            edge_by_node[neighbor] = e
            frontier.append((neighbor, depth + 1))

    return sorted(visited), _build_explanations(seeds_art, parents, edge_by_node)


def _build_explanations(
    seeds: Sequence[str],
    parents: Mapping[str, tuple[str, str]],
    edge_by_node: Mapping[str, ContextEdge],
) -> list[ContextExplanation]:
    explanations: list[ContextExplanation] = []
    seed_set = set(seeds)

    for target in sorted(parents.keys()):
        path = [target]
        edge_keys: list[str] = []
        cursor = target
        while cursor not in seed_set:
            parent = parents.get(cursor)
            if not parent:
                break
            parent_id, edge_key = parent
            edge_keys.append(edge_key)
            cursor = parent_id
            path.append(cursor)
        explanations.append(
            ContextExplanation(
                target_id=target,
                path=list(reversed(path)),
                edge_keys=list(reversed(edge_keys)),
            )
        )
    return explanations


def _fetch_fragment_ids(cx: "psycopg.Connection[Any]", artifact_ids: Sequence[str]) -> list[str]:
    rows = cx.execute(
        """
        SELECT fragment_id
          FROM ikam_artifact_fragments
         WHERE artifact_id = ANY(%s)
         ORDER BY artifact_id, position
        """,
        (list(artifact_ids),),
    ).fetchall()
    return [str(row["fragment_id"]) for row in rows]


# ---------------------------------------------------------------------------
# O6: Relational fragment proposal + validation
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class RelationProposal:
    artifact_id: str
    predicate: str
    subject_fragment_ids: Sequence[str]
    object_fragment_ids: Sequence[str]
    directed: bool = True
    confidence_score: float = 0.8
    qualifiers: Mapping[str, Any] = field(default_factory=dict)
    evidence_fragment_ids: Sequence[str] = field(default_factory=list)
    reasons: Sequence[str] = field(default_factory=list)


@dataclass(frozen=True)
class RelationValidationResult:
    accepted_fragments: list[V3Fragment]
    edge_events: list[KnowledgeEdgeEventInput]
    rejected: list[str]


def validate_relation_proposals(
    *,
    proposals: Sequence[RelationProposal],
    context_bundle: ContextBundle,
    max_reasons: int = 5,
) -> RelationValidationResult:
    """Validate relation proposals against the explicit context bundle."""
    if not proposals:
        raise ValueError("O6 requires at least one relation proposal")

    bundle_fragments = set(context_bundle.fragment_ids)
    must_link = set(tuple(pair) for pair in context_bundle.constraints.must_link)
    cannot_link = set(tuple(pair) for pair in context_bundle.constraints.cannot_link)

    accepted: list[V3Fragment] = []
    edges: list[KnowledgeEdgeEventInput] = []
    rejected: list[str] = []
    covered_must: set[tuple[str, str]] = set()

    for proposal in proposals:
        predicate = str(proposal.predicate).strip()
        if not predicate:
            rejected.append("predicate_missing")
            continue

        subjects = _unique_sorted(proposal.subject_fragment_ids)
        objects = _unique_sorted(proposal.object_fragment_ids)
        if not subjects or not objects:
            rejected.append(f"{predicate}:missing_endpoints")
            continue

        if not (0.0 <= float(proposal.confidence_score) <= 1.0):
            rejected.append(f"{predicate}:confidence_out_of_range")
            continue

        evidence_ids = _unique_sorted(proposal.evidence_fragment_ids)
        if not evidence_ids or not set(evidence_ids).issubset(bundle_fragments):
            rejected.append(f"{predicate}:evidence_outside_context")
            continue

        reasons = [str(r) for r in proposal.reasons[:max_reasons] if str(r).strip()]

        blocked = False
        for s in subjects:
            for o in objects:
                pair = (s, o)
                if pair in cannot_link:
                    rejected.append(f"{predicate}:cannot_link")
                    blocked = True
                if pair in must_link:
                    covered_must.add(pair)
        if blocked:
            continue

        content = Relation(
            predicate=predicate,
            directed=bool(proposal.directed),
            confidence_score=float(proposal.confidence_score),
            qualifiers={
                **dict(proposal.qualifiers or {}),
                "subject_fragment_ids": subjects,
                "object_fragment_ids": objects,
                "context_bundle_id": context_bundle.bundle_id,
                "evidence_fragment_ids": evidence_ids,
                "reasons": reasons,
            },
        )
        payload = json.dumps(content.model_dump(mode="json"), sort_keys=True).encode("utf-8")
        relation_fragment_id = _cas_hex(payload)
        fragment = V3Fragment(cas_id=relation_fragment_id, value=content, mime_type=RELATION_MIME)
        edge_inputs = build_knowledge_edge_events(
            op="upsert",
            relation_fragment_id=relation_fragment_id,
            predicate=predicate,
            subject_fragment_ids=subjects,
            object_fragment_ids=objects,
            directed=bool(proposal.directed),
            confidence_score=float(proposal.confidence_score),
            qualifiers=dict(proposal.qualifiers or {}),
        )

        accepted.append(fragment)
        edges.extend(edge_inputs)

    if must_link and must_link != covered_must:
        missing = sorted(must_link - covered_must)
        rejected.append(f"missing_must_link:{missing}")

    return RelationValidationResult(
        accepted_fragments=accepted,
        edge_events=edges,
        rejected=rejected,
    )
