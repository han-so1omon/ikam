from __future__ import annotations

from pathlib import Path
import sys
from unittest.mock import MagicMock, patch

import pytest

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/modelado/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))
sys.path.insert(0, str(ROOT / "packages/ikam/src"))


@patch("modelado.ikam_graph_repository.append_graph_edge_event")
@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_apply_graph_delta_envelope_appends_lowered_events(mock_require, mock_append) -> None:
    from modelado.ikam_graph_repository import apply_graph_delta_envelope

    cx = MagicMock()
    tx = MagicMock()
    cx.transaction.return_value.__enter__.return_value = tx
    cx.transaction.return_value.__exit__.return_value = False
    mock_append.side_effect = [
        {
            "id": 1,
            "project_id": "proj-123",
            "op": "upsert",
            "edge_label": "graph:value_at",
            "out_id": "graph-anchor:claim-set",
            "in_id": "graph-value:claim-set",
            "properties": {"derivationId": "graph-delta:proj-123:claim-set:[]"},
            "t": 1,
            "idempotency_key": "idem-1",
        },
        {
            "id": 2,
            "project_id": "proj-123",
            "op": "delete",
            "edge_label": "graph:value_at",
            "out_id": "graph-anchor:claim-set",
            "in_id": "graph-value:claim-set",
            "properties": {"derivationId": "graph-delta:proj-123:claim-set:[]"},
            "t": 2,
            "idempotency_key": "idem-2",
        },
    ]

    result = apply_graph_delta_envelope(
        cx,
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

    assert mock_append.call_count == 2
    cx.transaction.assert_called_once_with()
    first = mock_append.call_args_list[0].kwargs
    second = mock_append.call_args_list[1].kwargs
    assert first["project_id"] == "proj-123"
    assert first["op"] == "upsert"
    assert first["edge_label"] == "graph:value_at"
    assert second["op"] == "delete"
    assert result["summary"] == {
        "apply_mode": "atomic",
        "op_count": 2,
        "upsert_count": 1,
        "remove_count": 1,
    }
    assert len(result["appended_events"]) == 2


@patch("modelado.ikam_graph_repository.append_graph_edge_event")
@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_apply_graph_delta_envelope_keeps_idempotent_noops_in_result(mock_require, mock_append) -> None:
    from modelado.ikam_graph_repository import apply_graph_delta_envelope

    cx = MagicMock()
    cx.transaction.return_value.__enter__.return_value = cx
    cx.transaction.return_value.__exit__.return_value = False
    mock_append.return_value = None

    result = apply_graph_delta_envelope(
        cx,
        project_id="proj-123",
        envelope={
            "delta": {
                "ops": [
                    {
                        "op": "upsert",
                        "anchor": {"handle": "claim-set"},
                        "value": {"kind": "claim_set"},
                    }
                ]
            }
        },
    )

    assert result["summary"] == {
        "apply_mode": "atomic",
        "op_count": 1,
        "upsert_count": 1,
        "remove_count": 0,
    }
    assert result["appended_events"] == []
    assert result["attempted_event_count"] == 1


@patch("modelado.ikam_graph_repository.append_graph_edge_event")
@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_apply_graph_delta_envelope_raises_on_project_scope_mismatch(mock_require, mock_append) -> None:
    from modelado.core.execution_context import ExecutionContext, ExecutionMode, ExecutionPolicyViolation, WriteScope, execution_context
    from modelado.ikam_graph_repository import apply_graph_delta_envelope

    cx = MagicMock()
    scope = WriteScope(allowed=True, project_id="proj-scope", operation="graph-delta")

    with execution_context(ExecutionContext(mode=ExecutionMode.REQUEST, request_id="r1", actor_id=None, write_scope=scope)):
        with pytest.raises(ExecutionPolicyViolation, match="project_id"):
            apply_graph_delta_envelope(
                cx,
                project_id="proj-other",
                envelope={"delta": {"ops": []}},
            )


@patch("modelado.ikam_graph_repository.append_graph_edge_event")
@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_apply_graph_delta_envelope_raises_on_operation_scope_mismatch(mock_require, mock_append) -> None:
    from modelado.core.execution_context import ExecutionContext, ExecutionMode, ExecutionPolicyViolation, WriteScope, execution_context
    from modelado.ikam_graph_repository import apply_graph_delta_envelope

    cx = MagicMock()
    scope = WriteScope(allowed=True, project_id="proj-123", operation="other-operation")

    with execution_context(ExecutionContext(mode=ExecutionMode.REQUEST, request_id="r1", actor_id=None, write_scope=scope)):
        with pytest.raises(ExecutionPolicyViolation, match="operation"):
            apply_graph_delta_envelope(
                cx,
                project_id="proj-123",
                envelope={"delta": {"ops": []}},
            )


@patch("modelado.ikam_graph_repository.append_graph_edge_event")
@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_apply_graph_delta_envelope_propagates_append_failure_inside_transaction(mock_require, mock_append) -> None:
    from modelado.ikam_graph_repository import apply_graph_delta_envelope

    cx = MagicMock()
    cx.transaction.return_value.__enter__.return_value = cx
    cx.transaction.return_value.__exit__.return_value = False
    mock_append.side_effect = RuntimeError("append failed")

    with pytest.raises(RuntimeError, match="append failed"):
        apply_graph_delta_envelope(
            cx,
            project_id="proj-123",
            envelope={
                "delta": {
                    "ops": [
                        {
                            "op": "upsert",
                            "anchor": {"handle": "claim-set"},
                            "value": {"kind": "claim_set"},
                        }
                    ]
                }
            },
        )

    cx.transaction.assert_called_once_with()
