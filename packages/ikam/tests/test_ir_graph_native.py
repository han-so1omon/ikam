"""Tests for graph-native runtime contracts."""

import pytest
from pydantic import TypeAdapter, ValidationError


def test_graph_native_contracts_are_importable_from_package():
    from ikam.ir import (
        ATOMIC_GRAPH_APPLY_MODE,
        GRAPH_DELTA_MIME,
        GRAPH_SLICE_MIME,
        GraphAnchor,
        GraphRegion,
        IKAMGraphDelta,
        IKAMGraphDeltaOp,
        IKAMGraphSlice,
    )

    assert IKAMGraphSlice is not None
    assert IKAMGraphDelta is not None
    assert GraphAnchor is not None
    assert GraphRegion is not None
    assert IKAMGraphDeltaOp is not None
    assert ATOMIC_GRAPH_APPLY_MODE == "atomic"
    assert GRAPH_SLICE_MIME.endswith("+json")
    assert GRAPH_DELTA_MIME.endswith("+json")


def test_graph_anchor_and_region_use_runtime_handles_not_storage_graph_ids():
    from ikam.ir import GraphAnchor, GraphRegion

    anchor = GraphAnchor(handle="chapter:intro", path=("body", 0))
    region = GraphRegion(anchor=anchor, extent="subtree")

    assert anchor.handle == "chapter:intro"
    assert anchor.path == ("body", 0)
    assert region.anchor == anchor
    assert region.extent == "subtree"
    assert "graph_id" not in GraphAnchor.model_fields
    assert "graph_id" not in GraphRegion.model_fields


def test_graph_delta_op_union_is_tagged_and_scopes_destructive_ops_with_regions():
    from ikam.ir import IKAMGraphDeltaOp

    op = TypeAdapter(IKAMGraphDeltaOp).validate_python(
        {
            "op": "remove",
            "region": {
                "anchor": {"handle": "chapter:intro", "path": ["body", 0]},
                "extent": "node",
            },
        }
    )

    assert op.op == "remove"
    assert op.region.anchor.handle == "chapter:intro"


def test_graph_delta_rejects_non_atomic_apply_modes():
    from ikam.ir import IKAMGraphDelta

    with pytest.raises(ValidationError):
        IKAMGraphDelta.model_validate(
            {
                "ops": [],
                "apply_mode": "best_effort",
            }
        )


def test_graph_delta_defaults_to_atomic_mode_and_roundtrips():
    from ikam.ir import IKAMGraphDelta

    delta = IKAMGraphDelta.model_validate(
        {
            "ops": [
                {
                    "op": "upsert",
                    "anchor": {"handle": "chapter:intro"},
                    "value": {"text": "Hello"},
                }
            ]
        }
    )

    restored = IKAMGraphDelta.model_validate(delta.model_dump(mode="json"))

    assert delta.apply_mode == "atomic"
    assert restored == delta
    assert restored.ops[0].op == "upsert"


def test_graph_slice_captures_runtime_region_and_payload_metadata():
    from ikam.ir import GRAPH_SLICE_MIME, IKAMGraphSlice

    graph_slice = IKAMGraphSlice.model_validate(
        {
            "region": {
                "anchor": {"handle": "chapter:intro"},
                "extent": "subtree",
            },
            "payload": {"nodes": [{"handle": "chapter:intro"}]},
            "mime_type": GRAPH_SLICE_MIME,
        }
    )

    assert graph_slice.region.anchor.handle == "chapter:intro"
    assert graph_slice.mime_type == GRAPH_SLICE_MIME
    assert graph_slice.payload["nodes"][0]["handle"] == "chapter:intro"


def test_graph_slice_allows_non_object_runtime_payloads():
    from ikam.ir import GRAPH_SLICE_MIME, IKAMGraphSlice

    graph_slice = IKAMGraphSlice.model_validate(
        {
            "region": {
                "anchor": {"handle": "chapter:intro", "path": ["body", 0]},
                "extent": "node",
            },
            "payload": ["intro", "body"],
            "mime_type": GRAPH_SLICE_MIME,
        }
    )

    assert graph_slice.payload == ["intro", "body"]
