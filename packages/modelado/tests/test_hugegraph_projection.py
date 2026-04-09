from __future__ import annotations

from typing import Any

import pytest

from modelado.hugegraph_projection import (
    ensure_ikam_projection_schema,
    replay_graph_edge_events_until_done,
    replay_graph_edge_events_until_done_async,
)
from modelado.graph_edge_event_log import GraphEdgeEvent


class _FakeHugeGraphClient:
    def __init__(self) -> None:
        self.created: list[tuple[str, dict[str, Any]]] = []
        self.edges: list[dict[str, Any]] = []
        self.gremlin_queries: list[str] = []

    def schema_get(self, kind: str, name: str | None = None) -> Any:
        raise RuntimeError("not found")

    def schema_create(self, kind: str, payload: dict[str, Any]) -> Any:
        self.created.append((kind, payload))
        return payload

    def create_vertex(self, *, label: str, properties: dict[str, Any], vertex_id: str | None = None) -> Any:
        return {"id": vertex_id or f"{label}:{properties['id']}"}

    def create_edge(
        self,
        *,
        label: str,
        out_v: str,
        in_v: str,
        out_v_label: str,
        in_v_label: str,
        properties: dict[str, Any],
    ) -> Any:
        self.edges.append(
            {
                "label": label,
                "out_v": out_v,
                "in_v": in_v,
                "out_v_label": out_v_label,
                "in_v_label": in_v_label,
                "properties": properties,
            }
        )
        return {"id": "edge-1"}

    def gremlin_query(self, gremlin: str) -> Any:
        self.gremlin_queries.append(gremlin)
        return {"ok": True}


class _FakeConnection:
    def __init__(self) -> None:
        self.commit_calls = 0

    def commit(self) -> None:
        self.commit_calls += 1


def test_ensure_projection_schema_creates_pollock_and_computational_edges() -> None:
    client = _FakeHugeGraphClient()

    ensure_ikam_projection_schema(client)  # type: ignore[arg-type]

    edge_labels = {
        payload.get("name")
        for kind, payload in client.created
        if kind == "edgelabels"
    }
    assert "calculates" in edge_labels
    assert "feeds" in edge_labels
    assert "supports" in edge_labels
    assert "rebuts" in edge_labels
    assert "undercuts" in edge_labels
    assert "sequenced-in" in edge_labels


def test_ensure_projection_schema_creates_universal_glue_vertex_labels() -> None:
    client = _FakeHugeGraphClient()

    ensure_ikam_projection_schema(client)  # type: ignore[arg-type]

    vertex_labels = {
        payload.get("name")
        for kind, payload in client.created
        if kind == "vertexlabels"
    }
    assert "Artifact" in vertex_labels
    assert "Fragment" in vertex_labels
    assert "Proposition" in vertex_labels
    assert "Expression" in vertex_labels
    assert "StructuredData" in vertex_labels


def test_replay_maps_sequenced_in_to_first_class_edge_label(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    client = _FakeHugeGraphClient()
    cx = _FakeConnection()

    monkeypatch.setattr(
        "modelado.hugegraph_projection.get_projection_checkpoint",
        lambda *_args, **_kwargs: None,
    )
    monkeypatch.setattr(
        "modelado.hugegraph_projection.set_projection_checkpoint",
        lambda *_args, **_kwargs: None,
    )

    events = [
        GraphEdgeEvent(
            id=1,
            project_id="p1",
            op="upsert",
            edge_label="sequenced-in",
            out_id="prop:1",
            in_id="frag:1",
            properties={
                "out_properties": {"mime_type": "application/ikam-proposition+v1+json"},
                "in_properties": {"mime_type": "text/plain"},
            },
            t=1,
            idempotency_key="k1",
        )
    ]

    def _list_events(*_args: Any, **kwargs: Any) -> list[GraphEdgeEvent]:
        after_id = int(kwargs.get("after_id", 0))
        if after_id >= 1:
            return []
        return events

    monkeypatch.setattr("modelado.hugegraph_projection.list_graph_edge_events", _list_events)

    replay_graph_edge_events_until_done(
        cx,  # type: ignore[arg-type]
        client=client,  # type: ignore[arg-type]
        project_id="p1",
    )

    assert client.edges, "expected projected edges"
    assert client.edges[0]["label"] == "sequenced-in"


def test_projection_schema_and_replay_include_env_scope_properties(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    client = _FakeHugeGraphClient()
    cx = _FakeConnection()

    ensure_ikam_projection_schema(client)  # type: ignore[arg-type]

    property_keys = {
        payload.get("name")
        for kind, payload in client.created
        if kind == "propertykeys"
    }
    assert "envType" in property_keys
    assert "envId" in property_keys

    monkeypatch.setattr(
        "modelado.hugegraph_projection.get_projection_checkpoint",
        lambda *_args, **_kwargs: None,
    )
    monkeypatch.setattr(
        "modelado.hugegraph_projection.set_projection_checkpoint",
        lambda *_args, **_kwargs: None,
    )

    events = [
        GraphEdgeEvent(
            id=1,
            project_id="p1",
            op="upsert",
            edge_label="calculates",
            out_id="expr:1",
            in_id="frag:1",
            properties={
                "envType": "dev",
                "envId": "run/123",
                "out_properties": {"mime_type": "application/ikam-expression+v1+json"},
                "in_properties": {"mime_type": "application/ikam-structured-data+v1+json"},
            },
            t=1,
            idempotency_key="k1",
        )
    ]

    def _list_events(*_args: Any, **kwargs: Any) -> list[GraphEdgeEvent]:
        after_id = int(kwargs.get("after_id", 0))
        if after_id >= 1:
            return []
        return events

    monkeypatch.setattr("modelado.hugegraph_projection.list_graph_edge_events", _list_events)

    replay_graph_edge_events_until_done(
        cx,  # type: ignore[arg-type]
        client=client,  # type: ignore[arg-type]
        project_id="p1",
    )

    assert client.edges, "expected at least one projected edge"
    props = client.edges[0]["properties"]
    assert props["envType"] == "dev"
    assert props["envId"] == "run/123"


def test_replay_projects_subtree_graph_delete_by_handle_and_path_prefix(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    client = _FakeHugeGraphClient()
    cx = _FakeConnection()

    monkeypatch.setattr(
        "modelado.hugegraph_projection.get_projection_checkpoint",
        lambda *_args, **_kwargs: None,
    )
    monkeypatch.setattr(
        "modelado.hugegraph_projection.set_projection_checkpoint",
        lambda *_args, **_kwargs: None,
    )

    events = [
        GraphEdgeEvent(
            id=1,
            project_id="p1",
            op="upsert",
            edge_label="graph:value_at",
            out_id="graph-anchor:claim-set",
            in_id="graph-value:claim-set",
            properties={
                "graphDeltaHandle": "claim-set",
                "graphDeltaPath": ["claims", 0, "evidence"],
            },
            t=1,
            idempotency_key="k1",
        ),
        GraphEdgeEvent(
            id=2,
            project_id="p1",
            op="delete",
            edge_label="graph:value_at",
            out_id="graph-anchor:claim-set",
            in_id="graph-value:claim-set",
            properties={
                "graphDeltaHandle": "claim-set",
                "graphDeltaPath": ["claims", 0],
                "graphDeltaExtent": "subtree",
            },
            t=2,
            idempotency_key="k2",
        ),
    ]

    def _list_events(*_args: Any, **kwargs: Any) -> list[GraphEdgeEvent]:
        after_id = int(kwargs.get("after_id", 0))
        if after_id >= 2:
            return []
        return events

    monkeypatch.setattr("modelado.hugegraph_projection.list_graph_edge_events", _list_events)

    replay_graph_edge_events_until_done(
        cx,  # type: ignore[arg-type]
        client=client,  # type: ignore[arg-type]
        project_id="p1",
    )

    assert client.gremlin_queries, "expected subtree delete query"
    gremlin = client.gremlin_queries[0]
    assert "graphDeltaHandle', 'claim-set'" in gremlin
    assert "graphDeltaPath" in gremlin
    assert '[\"claims\",0' in gremlin


def test_replay_maps_graph_value_at_to_first_class_edge_label(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    client = _FakeHugeGraphClient()
    cx = _FakeConnection()

    monkeypatch.setattr(
        "modelado.hugegraph_projection.get_projection_checkpoint",
        lambda *_args, **_kwargs: None,
    )
    monkeypatch.setattr(
        "modelado.hugegraph_projection.set_projection_checkpoint",
        lambda *_args, **_kwargs: None,
    )

    events = [
        GraphEdgeEvent(
            id=1,
            project_id="p1",
            op="upsert",
            edge_label="graph:value_at",
            out_id="graph-anchor:claim-set",
            in_id="graph-value:claim-set",
            properties={
                "graphDeltaHandle": "claim-set",
                "graphDeltaPath": ["claims", 0],
            },
            t=1,
            idempotency_key="k1",
        )
    ]

    def _list_events(*_args: Any, **kwargs: Any) -> list[GraphEdgeEvent]:
        after_id = int(kwargs.get("after_id", 0))
        if after_id >= 1:
            return []
        return events

    monkeypatch.setattr("modelado.hugegraph_projection.list_graph_edge_events", _list_events)

    replay_graph_edge_events_until_done(
        cx,  # type: ignore[arg-type]
        client=client,  # type: ignore[arg-type]
        project_id="p1",
        edge_label_prefix="graph:",
    )

    assert client.edges, "expected projected graph value edge"
    assert client.edges[0]["label"] == "value-at"


@pytest.mark.anyio
async def test_async_projection_replay_delegates_to_sync(monkeypatch: pytest.MonkeyPatch) -> None:
    called: dict[str, Any] = {}

    def _fake_replay(*args: Any, **kwargs: Any) -> int:
        called["args"] = args
        called["kwargs"] = kwargs
        return 17

    monkeypatch.setattr(
        "modelado.hugegraph_projection.replay_graph_edge_events_until_done",
        _fake_replay,
    )

    result = await replay_graph_edge_events_until_done_async(
        cx="cx",
        client="client",
        project_id="project-1",
        consumer_name="projection-worker",
        edge_label_prefix="derivation:",
        batch_size=100,
    )

    assert result == 17
    assert called["kwargs"]["project_id"] == "project-1"
    assert called["kwargs"]["consumer_name"] == "projection-worker"
