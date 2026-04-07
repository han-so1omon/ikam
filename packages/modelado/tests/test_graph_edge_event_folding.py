from __future__ import annotations

from modelado.graph_edge_event_folding import fold_effective_edges
from modelado.graph_edge_event_log import (
    GraphEdgeEvent,
    compute_edge_identity_key,
    compute_relation_commit_receipt_id,
)


def _e(
    *,
    event_id: int,
    op: str,
    edge_label: str = "derivation:derived_from",
    out_id: str = "a",
    in_id: str = "b",
    properties: dict | None = None,
    t: int = 1700000000000,
    idempotency_key: str | None = None,
) -> GraphEdgeEvent:
    return GraphEdgeEvent(
        id=event_id,
        project_id="p",
        op=op,
        edge_label=edge_label,
        out_id=out_id,
        in_id=in_id,
        properties=dict(properties or {}),
        t=t,
        idempotency_key=idempotency_key,
    )


def test_fold_last_write_wins_per_effective_edge_identity() -> None:
    events = [
        _e(event_id=1, op="upsert", properties={"derivationId": "d1", "k": 1}, t=1),
        _e(event_id=2, op="upsert", properties={"derivationId": "d1", "k": 2}, t=2),
    ]

    effective = fold_effective_edges(events)
    assert len(effective) == 1

    key = compute_edge_identity_key(
        edge_label="derivation:derived_from",
        out_id="a",
        in_id="b",
        properties={"derivationId": "d1", "k": 2},
    )
    assert key in effective
    assert effective[key].properties["k"] == 2
    assert effective[key].last_event_id == 2


def test_fold_delete_removes_even_when_event_idempotency_keys_differ() -> None:
    upsert = _e(
        event_id=10,
        op="upsert",
        properties={"derivationId": "d9", "derivationType": "derived_from"},
        t=10,
        idempotency_key="idem-upsert",
    )
    delete = _e(
        event_id=11,
        op="delete",
        properties={"derivationId": "d9", "derivationType": "derived_from"},
        t=11,
        idempotency_key="idem-delete",
    )

    effective = fold_effective_edges([upsert, delete])
    assert effective == {}


def test_fold_multiple_edges_independent() -> None:
    events = [
        _e(event_id=1, op="upsert", out_id="a", in_id="b", properties={"derivationId": "d1"}, t=1),
        _e(event_id=2, op="upsert", out_id="a", in_id="c", properties={"derivationId": "d2"}, t=2),
        _e(event_id=3, op="delete", out_id="a", in_id="b", properties={"derivationId": "d1"}, t=3),
    ]

    effective = fold_effective_edges(events)
    assert len(effective) == 1
    remaining_key = compute_edge_identity_key(
        edge_label="derivation:derived_from",
        out_id="a",
        in_id="c",
        properties={"derivationId": "d2"},
    )
    assert remaining_key in effective


def test_relation_commit_receipt_id_is_deterministic_order_independent() -> None:
    first = compute_relation_commit_receipt_id(
        project_id="p1",
        overlay_id="rov-1",
        committed_fragment_ids=["f2", "f1"],
        edge_idempotency_keys=["e2", "e1"],
    )
    second = compute_relation_commit_receipt_id(
        project_id="p1",
        overlay_id="rov-1",
        committed_fragment_ids=["f1", "f2"],
        edge_idempotency_keys=["e1", "e2"],
    )
    assert first == second


def test_edge_identity_includes_environment_qualifiers() -> None:
    key_dev = compute_edge_identity_key(
        edge_label="knowledge:contains",
        out_id="a",
        in_id="b",
        properties={
            "relationFragmentId": "rel-1",
            "envType": "dev",
            "envId": "dev-1",
        },
    )
    key_staging = compute_edge_identity_key(
        edge_label="knowledge:contains",
        out_id="a",
        in_id="b",
        properties={
            "relationFragmentId": "rel-1",
            "envType": "staging",
            "envId": "stg-1",
        },
    )
    assert key_dev != key_staging
