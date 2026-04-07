"""Tests for updated provenance event writes with fragment_id and operation_id.

Validates that record_provenance_event() correctly passes the new columns
to the INSERT statement.
"""
import pytest
from unittest.mock import MagicMock, patch, ANY
import uuid


@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_provenance_event_includes_fragment_id(mock_require):
    """fragment_id is passed through to the INSERT."""
    from modelado.ikam_graph_repository import record_provenance_event

    cx = MagicMock()
    cursor = MagicMock()
    cx.cursor.return_value.__enter__ = MagicMock(return_value=cursor)
    cx.cursor.return_value.__exit__ = MagicMock(return_value=False)

    artifact_id = str(uuid.uuid4())

    record_provenance_event(
        cx,
        artifact_id=artifact_id,
        event_type="Created",
        fragment_id="cas_abc123",
        operation_id="op_001",
    )

    cursor.execute.assert_called_once()
    sql = cursor.execute.call_args[0][0]
    params = cursor.execute.call_args[0][1]

    assert "fragment_id" in sql
    assert "operation_id" in sql
    # params: (ev_id, artifact_id, derivation_id, event_type, author_id, details, fragment_id, operation_id)
    assert params[6] == "cas_abc123"  # fragment_id
    assert params[7] == "op_001"  # operation_id


@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_provenance_event_fragment_id_defaults_none(mock_require):
    """fragment_id and operation_id default to None when not provided."""
    from modelado.ikam_graph_repository import record_provenance_event

    cx = MagicMock()
    cursor = MagicMock()
    cx.cursor.return_value.__enter__ = MagicMock(return_value=cursor)
    cx.cursor.return_value.__exit__ = MagicMock(return_value=False)

    artifact_id = str(uuid.uuid4())

    record_provenance_event(
        cx,
        artifact_id=artifact_id,
        event_type="Created",
    )

    params = cursor.execute.call_args[0][1]
    assert params[6] is None  # fragment_id
    assert params[7] is None  # operation_id


@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_provenance_event_backward_compatible(mock_require):
    """Existing callers without fragment_id/operation_id still work."""
    from modelado.ikam_graph_repository import record_provenance_event

    cx = MagicMock()
    cursor = MagicMock()
    cx.cursor.return_value.__enter__ = MagicMock(return_value=cursor)
    cx.cursor.return_value.__exit__ = MagicMock(return_value=False)

    artifact_id = str(uuid.uuid4())
    author_id = str(uuid.uuid4())

    ev_id = record_provenance_event(
        cx,
        artifact_id=artifact_id,
        event_type="Modified",
        author_id=author_id,
        details={"change": "updated"},
    )

    # Should return a valid UUID string
    uuid.UUID(ev_id)

    params = cursor.execute.call_args[0][1]
    assert params[3] == "Modified"  # event_type
    assert params[6] is None  # fragment_id
    assert params[7] is None  # operation_id
