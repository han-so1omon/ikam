from __future__ import annotations

from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/modelado/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))
sys.path.insert(0, str(ROOT / "packages/ikam/src"))


def test_lower_graph_delta_envelope_emits_deterministic_graph_edge_events() -> None:
    from modelado.graph.delta_lowering import lower_graph_delta_envelope

    lowered = lower_graph_delta_envelope(
        project_id="proj-123",
        envelope={
            "delta": {
                "ops": [
                    {
                        "op": "upsert",
                        "anchor": {"handle": "claim-set"},
                        "value": {"kind": "claim_set", "claim_refs": ["frag-1"]},
                    },
                    {
                        "op": "remove",
                        "region": {
                            "anchor": {"handle": "stale-claim-set"},
                            "extent": "subtree",
                        },
                    },
                ]
            }
        },
    )

    assert lowered.summary == {
        "apply_mode": "atomic",
        "op_count": 2,
        "upsert_count": 1,
        "remove_count": 1,
    }
    assert [event.edge_label for event in lowered.edge_events] == ["graph:value_at", "graph:value_at"]
    assert [event.op for event in lowered.edge_events] == ["upsert", "delete"]
    assert lowered.edge_events[0].out_id == "graph-anchor:claim-set"
    assert lowered.edge_events[0].in_id == "graph-value:claim-set"
    assert lowered.edge_events[0].properties == {
        "derivationId": "graph-delta:proj-123:claim-set:[]",
        "graphDeltaHandle": "claim-set",
        "graphDeltaPath": [],
        "graphDeltaValue": {
            "kind": "claim_set",
            "claim_refs": ["frag-1"],
        },
    }
    assert lowered.edge_events[1].out_id == "graph-anchor:stale-claim-set"
    assert lowered.edge_events[1].in_id == "graph-value:stale-claim-set"
    assert lowered.edge_events[1].properties == {
        "derivationId": "graph-delta:proj-123:stale-claim-set:[]",
        "graphDeltaHandle": "stale-claim-set",
        "graphDeltaPath": [],
        "graphDeltaExtent": "subtree",
    }


def test_lower_graph_delta_remove_targets_same_effective_edge_identity_as_upsert() -> None:
    from modelado.graph.delta_lowering import lower_graph_delta_envelope
    from modelado.graph_edge_event_folding import fold_effective_edges
    from modelado.graph_edge_event_log import GraphEdgeEvent

    lowered = lower_graph_delta_envelope(
        project_id="proj-123",
        envelope={
            "delta": {
                "ops": [
                    {
                        "op": "upsert",
                        "anchor": {"handle": "claim-set"},
                        "value": {"kind": "claim_set"},
                    },
                    {
                        "op": "remove",
                        "region": {
                            "anchor": {"handle": "claim-set"},
                            "extent": "subtree",
                        },
                    },
                ]
            }
        },
    )

    events = [
        GraphEdgeEvent(
            id=index,
            project_id="proj-123",
            op=edge_event.op,
            edge_label=edge_event.edge_label,
            out_id=edge_event.out_id,
            in_id=edge_event.in_id,
            properties=edge_event.properties,
            t=index,
            idempotency_key=edge_event.idempotency_key,
        )
        for index, edge_event in enumerate(lowered.edge_events, start=1)
    ]

    assert fold_effective_edges(events) == {}


def test_lower_graph_delta_upserts_with_different_values_produce_distinct_idempotent_events() -> None:
    from modelado.graph.delta_lowering import lower_graph_delta_envelope

    first = lower_graph_delta_envelope(
        project_id="proj-123",
        envelope={
            "delta": {
                "ops": [
                    {
                        "op": "upsert",
                        "anchor": {"handle": "claim-set"},
                        "value": {"kind": "claim_set", "claim_refs": ["frag-1"]},
                    }
                ]
            }
        },
    )
    second = lower_graph_delta_envelope(
        project_id="proj-123",
        envelope={
            "delta": {
                "ops": [
                    {
                        "op": "upsert",
                        "anchor": {"handle": "claim-set"},
                        "value": {"kind": "claim_set", "claim_refs": ["frag-2"]},
                    }
                ]
            }
        },
    )

    assert first.edge_events[0].edge_identity_key == second.edge_events[0].edge_identity_key
    assert first.edge_events[0].idempotency_key != second.edge_events[0].idempotency_key


def test_lower_graph_delta_envelope_is_stable_for_equivalent_inputs() -> None:
    from modelado.graph.delta_lowering import lower_graph_delta_envelope

    first_payload = {
        "delta": {
            "ops": [
                {
                    "op": "upsert",
                    "anchor": {"handle": "claim-set", "path": ["claims", 0]},
                    "value": {"kind": "claim_set", "claim_refs": ["frag-1"], "source": {"run": "r1"}},
                }
            ]
        }
    }
    second_payload = {
        "delta": {
            "ops": [
                {
                    "value": {"source": {"run": "r1"}, "claim_refs": ["frag-1"], "kind": "claim_set"},
                    "anchor": {"path": ["claims", 0], "handle": "claim-set"},
                    "op": "upsert",
                }
            ]
        }
    }

    first = lower_graph_delta_envelope(project_id="proj-123", envelope=first_payload)
    second = lower_graph_delta_envelope(project_id="proj-123", envelope=second_payload)

    assert first.edge_events == second.edge_events
    assert first.summary == second.summary


def test_lower_graph_delta_envelope_rejects_empty_handle() -> None:
    import pytest

    from modelado.graph.delta_lowering import lower_graph_delta_envelope

    with pytest.raises(ValueError, match="handle"):
        lower_graph_delta_envelope(
            project_id="proj-123",
            envelope={
                "delta": {
                    "ops": [
                        {
                            "op": "upsert",
                            "anchor": {"handle": "   "},
                            "value": {"kind": "claim_set"},
                        }
                    ]
                }
            },
        )


def test_lower_graph_delta_envelope_rejects_empty_remove_handle() -> None:
    import pytest

    from modelado.graph.delta_lowering import lower_graph_delta_envelope

    with pytest.raises(ValueError, match="handle"):
        lower_graph_delta_envelope(
            project_id="proj-123",
            envelope={
                "delta": {
                    "ops": [
                        {
                            "op": "remove",
                            "region": {
                                "anchor": {"handle": "   "},
                                "extent": "subtree",
                            },
                        }
                    ]
                }
            },
        )


def test_lower_graph_delta_envelope_rejects_non_json_serializable_values() -> None:
    import pytest

    from modelado.graph.delta_lowering import lower_graph_delta_envelope

    with pytest.raises(ValueError, match="JSON-serializable"):
        lower_graph_delta_envelope(
            project_id="proj-123",
            envelope={
                "delta": {
                    "ops": [
                        {
                            "op": "upsert",
                            "anchor": {"handle": "claim-set"},
                            "value": {"kind": "claim_set", "bad": {1, 2, 3}},
                        }
                    ]
                }
            },
        )
