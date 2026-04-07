from __future__ import annotations

from modelado.graph_edge_event_log import GraphEdgeEvent
from modelado.ikam_operator_surface import (
    KindCandidate,
    infer_artifact_kind,
    decompose_bytes,
    canonicalize_and_store_fragments,
    build_context_bundle,
    build_realization_record,
    RealizationRecord,
    TraversalPolicy,
    ContextConstraints,
    RelationProposal,
    validate_relation_proposals,
)


def test_infer_artifact_kind_tie_breaks_lexicographically() -> None:
    result = infer_artifact_kind(
        [
            KindCandidate(kind="sheet", confidence=0.8, evidence={"k": 1}),
            KindCandidate(kind="document", confidence=0.8, evidence={"k": 2}),
        ]
    )
    assert result.kind == "document"
    assert result.confidence == 0.8
    assert result.evidence["k"] == 2
    assert result.evidence_hash


def test_decompose_bytes_creates_binary_root_fragment() -> None:
    payload = b"hello"
    result = decompose_bytes(
        artifact_id="a-1",
        payload=payload,
        mime_type="text/plain",
        metadata={"source": "test"},
    )
    assert len(result.fragments) == 1
    frag = result.fragments[0]
    assert frag.cas_id
    assert frag.mime_type == "text/plain"
    assert frag.value == payload


def test_canonicalize_and_store_fragments_without_db() -> None:
    payload = b"bytes"
    decomp = decompose_bytes(
        artifact_id="a-2",
        payload=payload,
        mime_type="application/octet-stream",
    )
    result = canonicalize_and_store_fragments(fragments=decomp.fragments, cx=None)
    frag = decomp.fragments[0]
    assert frag.cas_id in result.domain_id_to_cas_id
    assert result.cas_ids == [frag.cas_id]


def _event(event_id: int, out_id: str, in_id: str) -> GraphEdgeEvent:
    return GraphEdgeEvent(
        id=event_id,
        project_id="p1",
        op="upsert",
        edge_label="derivation:derived_from",
        out_id=out_id,
        in_id=in_id,
        properties={"derivationId": f"d{event_id}"},
        t=1700000000000 + event_id,
        idempotency_key=f"idem-{event_id}",
    )


def test_build_context_bundle_deterministic_with_events() -> None:
    events = [_event(2, "b", "c"), _event(1, "a", "b")]
    policy = TraversalPolicy(max_depth=2, include_upstream=False, include_downstream=True)

    bundle1 = build_context_bundle(
        project_id="p1",
        seed_artifact_ids=["a"],
        edge_events=events,
        policy=policy,
    )
    bundle2 = build_context_bundle(
        project_id="p1",
        seed_artifact_ids=["a"],
        edge_events=list(reversed(events)),
        policy=policy,
    )

    assert bundle1.artifact_ids == ["a", "b", "c"]
    assert bundle1.bundle_id == bundle2.bundle_id
    assert [e.edge_key for e in bundle1.edges] == [e.edge_key for e in bundle2.edges]


def test_validate_relation_proposals_requires_context_evidence() -> None:
    bundle = build_context_bundle(
        project_id="p2",
        seed_artifact_ids=["a"],
        seed_fragment_ids=["f1", "f2"],
        edge_events=[],
        policy=TraversalPolicy(),
        constraints=ContextConstraints(),
    )

    ok = RelationProposal(
        artifact_id="a",
        predicate="supports",
        subject_fragment_ids=["f1"],
        object_fragment_ids=["f2"],
        evidence_fragment_ids=["f1"],
        reasons=["evidence says so"],
    )
    bad = RelationProposal(
        artifact_id="a",
        predicate="contradicts",
        subject_fragment_ids=["f1"],
        object_fragment_ids=["f2"],
        evidence_fragment_ids=["outside"],
    )

    result = validate_relation_proposals(
        proposals=[ok, bad],
        context_bundle=bundle,
    )

    assert len(result.accepted_fragments) == 1
    assert result.rejected
    assert all(e.edge_label.startswith("knowledge:") for e in result.edge_events)


def test_effect_realization_record_is_complete_and_replayable() -> None:
    """Contract D1, D2, D3: effectful operations produce complete, replayable realization records."""
    # --- Execute effectful operation: decompose + canonicalize ---
    payload = b"realization test content"
    decomp = decompose_bytes(
        artifact_id="art-replay",
        payload=payload,
        mime_type="text/plain",
        strategy="opaque_binary",
    )
    cas_result = canonicalize_and_store_fragments(fragments=decomp.fragments, cx=None)

    # --- Build realization record ---
    record = build_realization_record(
        operator_spec={"name": "decompose_and_store", "version": "1.0.0"},
        inputs={"artifact_id": "art-replay", "mime_type": "text/plain"},
        policy={"strategy": "opaque_binary"},
        exogenous={"env": "test"},
        outcome_fragments=decomp.fragments,
    )

    # --- D2: Realization Record Completeness ---
    assert isinstance(record, RealizationRecord)
    assert record.operator_spec == {"name": "decompose_and_store", "version": "1.0.0"}
    assert record.inputs == {"artifact_id": "art-replay", "mime_type": "text/plain"}
    assert record.policy == {"strategy": "opaque_binary"}
    assert record.exogenous_fingerprint  # non-empty hash string
    assert record.outcome_fid  # D1: canonical CAS identity for outcome

    # --- D1: outcome_fid is a canonical CAS ID ---
    expected_cas_id = decomp.fragments[0].cas_id
    assert record.outcome_fid == expected_cas_id

    # --- D3: Replay determinism — same inputs produce identical record ---
    replay = build_realization_record(
        operator_spec={"name": "decompose_and_store", "version": "1.0.0"},
        inputs={"artifact_id": "art-replay", "mime_type": "text/plain"},
        policy={"strategy": "opaque_binary"},
        exogenous={"env": "test"},
        outcome_fragments=decomp.fragments,
    )
    assert replay.outcome_fid == record.outcome_fid
    assert replay.exogenous_fingerprint == record.exogenous_fingerprint
    assert replay.record_id == record.record_id  # deterministic record identity
