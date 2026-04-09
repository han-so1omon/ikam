from __future__ import annotations

from unittest.mock import MagicMock, patch

from modelado.ikam_graph_repository import emit_edge_event
from modelado.graph_edge_event_log import GraphEdgeEvent
from modelado.graph_edge_projection_replay import ProjectionPolicy, replay_effective_edges


def _event(
    *,
    event_id: int,
    op: str,
    edge_label: str = "derivation:derived_from",
    out_id: str = "a",
    in_id: str = "b",
    properties: dict | None = None,
    t: int = 1700000000000,
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
        idempotency_key=None,
    )


def test_replay_deterministic_across_runs() -> None:
    events = [
        _event(event_id=1, op="upsert", edge_label="knowledge:derived_from", properties={"derivationId": "d1"}, t=1),
        _event(event_id=2, op="upsert", edge_label="knowledge:derived_from", properties={"derivationId": "d1", "k": 2}, t=2),
    ]
    policy = ProjectionPolicy(edge_label_prefix="knowledge:")

    snapshot_a = replay_effective_edges(events, policy=policy)
    snapshot_b = replay_effective_edges(list(events), policy=policy)

    assert snapshot_a.effective_edges == snapshot_b.effective_edges
    assert snapshot_a.last_event_id == snapshot_b.last_event_id


def test_replay_chunked_matches_full_replay() -> None:
    events = [
        _event(event_id=1, op="upsert", edge_label="knowledge:derived_from", properties={"derivationId": "d1"}, t=1),
        _event(event_id=2, op="upsert", edge_label="knowledge:derived_from", properties={"derivationId": "d2"}, out_id="a", in_id="c", t=2),
        _event(event_id=3, op="delete", edge_label="knowledge:derived_from", properties={"derivationId": "d1"}, t=3),
    ]
    policy = ProjectionPolicy(edge_label_prefix="knowledge:")

    full = replay_effective_edges(events, policy=policy)
    first = replay_effective_edges(events[:2], policy=policy)
    chunked = replay_effective_edges(events[2:], policy=policy, base=first.effective_edges)

    assert full.effective_edges == chunked.effective_edges
    assert full.last_event_id == chunked.last_event_id


def test_policy_filters_edges_but_checkpoint_advances() -> None:
    events = [
        _event(event_id=10, op="upsert", edge_label="knowledge:derived_from", properties={"derivationId": "d1"}, t=10),
        _event(
            event_id=11,
            op="upsert",
            edge_label="knowledge:related_to",
            properties={"relationFragmentId": "rel-1"},
            t=11,
        ),
    ]
    policy = ProjectionPolicy(edge_label_prefix="knowledge:")

    snapshot = replay_effective_edges(events, policy=policy)

    assert len(snapshot.effective_edges) == 2
    assert snapshot.last_event_id == 11


def test_policy_ignores_malformed_legacy_scope_qualifiers() -> None:
    events = [
        _event(
            event_id=1,
            op="upsert",
            edge_label="knowledge:contains",
            properties={"relationFragmentId": "bad-scope", "envType": "dev"},
            t=1,
        ),
        _event(
            event_id=2,
            op="upsert",
            edge_label="knowledge:contains",
            properties={"relationFragmentId": "good-scope", "ref": "refs/heads/main"},
            t=2,
        ),
    ]

    snapshot = replay_effective_edges(events, policy=ProjectionPolicy(edge_label_prefix="knowledge:"))

    assert len(snapshot.effective_edges) == 2


def test_policy_filters_by_environment_scope() -> None:
    events = [
        _event(
            event_id=1,
            op="upsert",
            edge_label="knowledge:contains",
            properties={"relationFragmentId": "rel-1", "envType": "dev", "envId": "dev-1"},
            t=1,
        ),
        _event(
            event_id=2,
            op="upsert",
            edge_label="knowledge:contains",
            properties={"relationFragmentId": "rel-1", "envType": "staging", "envId": "stg-1"},
            t=2,
        ),
    ]
    policy = ProjectionPolicy(edge_label_prefix="knowledge:", env_type="dev", env_id="dev-1")
    snapshot = replay_effective_edges(events, policy=policy)
    assert len(snapshot.effective_edges) == 1
    only = next(iter(snapshot.effective_edges.values()))
    assert only.properties["envType"] == "dev"
    assert only.properties["envId"] == "dev-1"


@patch("modelado.ikam_graph_repository.append_graph_edge_event")
@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_emit_edge_event_qualifiers_include_canonical_ref(mock_require, mock_append) -> None:
    cx = MagicMock()

    emit_edge_event(
        cx,
        source_id="a",
        target_id="b",
        predicate="contains",
        project_id="proj1",
        ref="refs/heads/run/run-123",
    )

    properties = mock_append.call_args.kwargs["properties"]
    assert properties["ref"] == "refs/heads/run/run-123"
    assert "envType" not in properties
    assert "envId" not in properties


def test_policy_filters_by_ref_without_legacy_env_fallback() -> None:
    events = [
        _event(
            event_id=1,
            op="upsert",
            edge_label="knowledge:contains",
            out_id="a",
            in_id="b",
            properties={
                "relationFragmentId": "rel-new",
                "ref": "refs/heads/run/run-123",
            },
            t=1,
        ),
        _event(
            event_id=3,
            op="upsert",
            edge_label="knowledge:contains",
            out_id="a",
            in_id="d",
            properties={
                "relationFragmentId": "rel-other",
                "ref": "refs/heads/main",
            },
            t=3,
        ),
    ]

    snapshot = replay_effective_edges(
        events,
        policy=ProjectionPolicy(edge_label_prefix="knowledge:", ref="refs/heads/run/run-123"),
    )

    assert len(snapshot.effective_edges) == 1
    relation_ids = {edge.properties["relationFragmentId"] for edge in snapshot.effective_edges.values()}
    assert relation_ids == {"rel-new"}


def test_policy_filters_by_pipeline_identity() -> None:
    events = [
        _event(
            event_id=1,
            op="upsert",
            edge_label="knowledge:contains",
            properties={
                "relationFragmentId": "rel-1",
                "pipelineId": "compression-rerender/v1",
                "pipelineRunId": "pipe-a",
            },
            t=1,
        ),
        _event(
            event_id=2,
            op="upsert",
            edge_label="knowledge:contains",
            properties={
                "relationFragmentId": "rel-1",
                "pipelineId": "compression-rerender/v1",
                "pipelineRunId": "pipe-b",
            },
            t=2,
        ),
    ]
    policy = ProjectionPolicy(
        edge_label_prefix="knowledge:",
        pipeline_id="compression-rerender/v1",
        pipeline_run_id="pipe-a",
    )
    snapshot = replay_effective_edges(events, policy=policy)
    assert len(snapshot.effective_edges) == 1
    only = next(iter(snapshot.effective_edges.values()))
    assert only.properties["pipelineId"] == "compression-rerender/v1"
    assert only.properties["pipelineRunId"] == "pipe-a"


def test_replay_subtree_delete_removes_descendant_graph_contains_edges() -> None:
    events = [
        _event(
            event_id=1,
            op="upsert",
            edge_label="graph:value_at",
            out_id="graph-anchor:claim-set",
            in_id="graph-value:claim-set",
            properties={
                "derivationId": "graph-delta:p:claim-set:[\"claims\",0]",
                "graphDeltaHandle": "claim-set",
                "graphDeltaPath": ["claims", 0],
                "graphDeltaValue": {"kind": "claim"},
            },
            t=1,
        ),
        _event(
            event_id=2,
            op="upsert",
            edge_label="graph:value_at",
            out_id="graph-anchor:claim-set",
            in_id="graph-value:claim-set",
            properties={
                "derivationId": "graph-delta:p:claim-set:[\"claims\",0,\"evidence\"]",
                "graphDeltaHandle": "claim-set",
                "graphDeltaPath": ["claims", 0, "evidence"],
                "graphDeltaValue": {"kind": "evidence"},
            },
            t=2,
        ),
        _event(
            event_id=3,
            op="delete",
            edge_label="graph:value_at",
            out_id="graph-anchor:claim-set",
            in_id="graph-value:claim-set",
            properties={
                "derivationId": "graph-delta:p:claim-set:[\"claims\",0]",
                "graphDeltaHandle": "claim-set",
                "graphDeltaPath": ["claims", 0],
                "graphDeltaExtent": "subtree",
            },
            t=3,
        ),
    ]

    snapshot = replay_effective_edges(events, policy=ProjectionPolicy(edge_label_prefix="graph:"))

    assert snapshot.effective_edges == {}
