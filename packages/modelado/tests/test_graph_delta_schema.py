from __future__ import annotations

from pathlib import Path
import sys

import pytest

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/modelado/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))
sys.path.insert(0, str(ROOT / "packages/ikam/src"))

from ikam.ir import GRAPH_DELTA_MIME, GRAPH_SLICE_MIME


def test_graph_delta_envelope_round_trips_with_atomic_runtime_delta() -> None:
    from modelado.graph.delta_schema import GRAPH_DELTA_ENVELOPE_SCHEMA_ID, IKAMGraphDeltaEnvelope

    envelope = IKAMGraphDeltaEnvelope.model_validate(
        {
            "schema": GRAPH_DELTA_ENVELOPE_SCHEMA_ID,
            "delta": {
                "ops": [
                    {
                        "op": "upsert",
                        "anchor": {"handle": "chunk-extraction-set"},
                        "value": {"kind": "chunk_extraction_set"},
                    },
                    {
                        "op": "remove",
                        "region": {
                            "anchor": {"handle": "stale-claim-set"},
                            "extent": "subtree",
                        },
                    },
                ],
                "apply_mode": "atomic",
            },
        }
    )

    dumped = envelope.model_dump(mode="json", by_alias=True)

    assert dumped == {
        "schema": GRAPH_DELTA_ENVELOPE_SCHEMA_ID,
        "mime_type": GRAPH_DELTA_MIME,
        "delta": {
            "ops": [
                {
                    "op": "upsert",
                    "anchor": {"handle": "chunk-extraction-set", "path": []},
                    "value": {"kind": "chunk_extraction_set"},
                },
                {
                    "op": "remove",
                    "region": {
                        "anchor": {"handle": "stale-claim-set", "path": []},
                        "extent": "subtree",
                    },
                },
            ],
            "apply_mode": "atomic",
        },
    }


def test_graph_delta_envelope_uses_graph_delta_mime_type() -> None:
    from modelado.graph.delta_schema import IKAMGraphDeltaEnvelope

    envelope = IKAMGraphDeltaEnvelope.model_validate({"delta": {"ops": []}})

    assert envelope.mime_type == GRAPH_DELTA_MIME


def test_graph_delta_envelope_requires_ops_field_to_match_runtime_contract() -> None:
    from pydantic import ValidationError
    from modelado.graph.delta_schema import IKAMGraphDeltaEnvelope

    with pytest.raises(ValidationError):
        IKAMGraphDeltaEnvelope.model_validate({"delta": {}})


def test_graph_delta_envelope_rejects_non_atomic_apply_mode() -> None:
    from pydantic import ValidationError
    from modelado.graph.delta_schema import IKAMGraphDeltaEnvelope

    with pytest.raises(ValidationError):
        IKAMGraphDeltaEnvelope.model_validate(
            {
                "delta": {
                    "ops": [],
                    "apply_mode": "best_effort",
                }
            }
        )


def test_graph_delta_envelope_rejects_invalid_schema_and_mime_type() -> None:
    from pydantic import ValidationError
    from modelado.graph.delta_schema import IKAMGraphDeltaEnvelope

    with pytest.raises(ValidationError):
        IKAMGraphDeltaEnvelope.model_validate(
            {
                "schema": "modelado/not-graph-delta@1",
                "delta": {"ops": []},
            }
        )

    with pytest.raises(ValidationError):
        IKAMGraphDeltaEnvelope.model_validate(
            {
                "mime_type": "application/not-a-graph-delta+json",
                "delta": {"ops": []},
            }
        )


def test_graph_delta_envelope_rejects_extra_envelope_fields() -> None:
    from pydantic import ValidationError
    from modelado.graph.delta_schema import IKAMGraphDeltaEnvelope

    with pytest.raises(ValidationError):
        IKAMGraphDeltaEnvelope.model_validate(
            {
                "delta": {"ops": []},
                "delta_id": "delta:test",
            }
        )


def test_graph_delta_envelope_rejects_nested_extra_delta_fields() -> None:
    from pydantic import ValidationError
    from modelado.graph.delta_schema import IKAMGraphDeltaEnvelope

    with pytest.raises(ValidationError):
        IKAMGraphDeltaEnvelope.model_validate(
            {
                "delta": {
                    "ops": [
                        {
                            "op": "upsert",
                            "anchor": {"handle": "claim-set", "unexpected": True},
                            "value": {"kind": "claim_set"},
                        }
                    ]
                }
            }
        )


def test_graph_delta_envelope_does_not_model_base_slice_mime_type() -> None:
    from pydantic import ValidationError
    from modelado.graph.delta_schema import IKAMGraphDeltaEnvelope

    with pytest.raises(ValidationError):
        IKAMGraphDeltaEnvelope.model_validate(
            {
                "delta": {"ops": []},
                "base_slice_mime_type": GRAPH_SLICE_MIME,
            }
        )
