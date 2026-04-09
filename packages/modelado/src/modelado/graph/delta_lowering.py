from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any

from modelado.graph.delta_schema import IKAMGraphDeltaEnvelope
from modelado.knowledge_edge_events import KnowledgeEdgeEventInput
from modelado.graph_edge_event_log import compute_edge_event_idempotency_key, compute_edge_identity_key


@dataclass(frozen=True)
class LoweredGraphDelta:
    edge_events: list[KnowledgeEdgeEventInput]
    summary: dict[str, Any]


def lower_graph_delta_envelope(*, project_id: str, envelope: IKAMGraphDeltaEnvelope | dict[str, object]) -> LoweredGraphDelta:
    validated = envelope if isinstance(envelope, IKAMGraphDeltaEnvelope) else IKAMGraphDeltaEnvelope.model_validate(envelope)

    edge_events: list[KnowledgeEdgeEventInput] = []
    upsert_count = 0
    remove_count = 0
    for op in validated.delta.ops:
        if op.op == "upsert":
            handle = op.anchor.handle.strip()
            if not handle:
                raise ValueError("handle must not be empty")
            derivation_id = _graph_delta_derivation_id(project_id=project_id, handle=handle, path=op.anchor.path)
            properties = {
                "derivationId": derivation_id,
                "graphDeltaHandle": handle,
                "graphDeltaPath": list(op.anchor.path),
                "graphDeltaValue": op.value,
            }
            _assert_json_serializable(op.value)
            edge_events.append(
                _build_edge_event(
                    op="upsert",
                    edge_label="graph:value_at",
                    out_id=f"graph-anchor:{handle}",
                    in_id=f"graph-value:{handle}",
                    properties=properties,
                )
            )
            upsert_count += 1
            continue

        handle = op.region.anchor.handle.strip()
        if not handle:
            raise ValueError("handle must not be empty")
        derivation_id = _graph_delta_derivation_id(project_id=project_id, handle=handle, path=op.region.anchor.path)
        properties = {
            "derivationId": derivation_id,
            "graphDeltaHandle": handle,
            "graphDeltaPath": list(op.region.anchor.path),
            "graphDeltaExtent": op.region.extent,
        }
        edge_events.append(
            _build_edge_event(
                op="delete",
                edge_label="graph:value_at",
                out_id=f"graph-anchor:{handle}",
                in_id=f"graph-value:{handle}",
                properties=properties,
            )
        )
        remove_count += 1

    return LoweredGraphDelta(
        edge_events=edge_events,
        summary={
            "apply_mode": validated.delta.apply_mode,
            "op_count": len(validated.delta.ops),
            "upsert_count": upsert_count,
            "remove_count": remove_count,
        },
    )


def _build_edge_event(*, op: str, edge_label: str, out_id: str, in_id: str, properties: dict[str, Any]) -> KnowledgeEdgeEventInput:
    canonical_properties = _canonical_json_properties(properties)
    return KnowledgeEdgeEventInput(
        op=op,
        edge_label=edge_label,
        out_id=out_id,
        in_id=in_id,
        properties=canonical_properties,
        idempotency_key=compute_edge_event_idempotency_key(
            op=op,
            edge_label=edge_label,
            out_id=out_id,
            in_id=in_id,
            properties=canonical_properties,
        ),
        edge_identity_key=compute_edge_identity_key(
            edge_label=edge_label,
            out_id=out_id,
            in_id=in_id,
            properties=canonical_properties,
        ),
    )


def _canonical_json_properties(properties: dict[str, Any]) -> dict[str, Any]:
    try:
        return json.loads(json.dumps(properties, sort_keys=True, separators=(",", ":"), ensure_ascii=False))
    except TypeError as exc:
        raise ValueError("graph delta lowering requires JSON-serializable values") from exc


def _assert_json_serializable(value: Any) -> None:
    try:
        json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    except TypeError as exc:
        raise ValueError("graph delta lowering requires JSON-serializable values") from exc


def _graph_delta_derivation_id(*, project_id: str, handle: str, path: tuple[str | int, ...]) -> str:
    return f"graph-delta:{project_id}:{handle}:{json.dumps(list(path), separators=(',', ':'), ensure_ascii=False)}"
