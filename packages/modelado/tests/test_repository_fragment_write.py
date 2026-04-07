"""Tests for rewritten fragment write path using ikam_fragment_store.

These are integration tests requiring a running PostgreSQL with the
ikam_fragment_store table. They validate CAS deduplication and env partitioning.
"""
import pytest
from unittest.mock import MagicMock, patch
import json


def _make_mock_connection():
    """Create a mock psycopg connection with cursor context manager."""
    cx = MagicMock()
    cursor = MagicMock()
    cx.cursor.return_value.__enter__ = MagicMock(return_value=cursor)
    cx.cursor.return_value.__exit__ = MagicMock(return_value=False)
    return cx, cursor


@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_store_fragment_calls_insert_with_canonical_ref(mock_require):
    """store_fragment inserts into ikam_fragment_store keyed by canonical ref."""
    from modelado.ikam_graph_repository import store_fragment
    from ikam.forja.cas import cas_fragment

    cx, cursor = _make_mock_connection()
    frag = cas_fragment({"text": "test"}, "text/ikam-paragraph")
    ref = "refs/heads/run/run-123"

    store_fragment(cx, frag, project_id="proj1", ref=ref)

    # Verify INSERT was called
    cursor.execute.assert_called_once()
    sql = cursor.execute.call_args[0][0]
    params = cursor.execute.call_args[0][1]

    assert "ikam_fragment_store" in sql
    assert "ON CONFLICT" in sql
    assert "(cas_id, ref, COALESCE(operation_id, ''))" in sql
    assert params[0] == frag.cas_id
    assert params[1] == ref
    assert params[3] == "proj1"
    assert params[4] == json.dumps({"text": "test"})
    assert params[5] == "text/ikam-paragraph"


@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_store_fragment_requires_cas_id(mock_require):
    """store_fragment raises ValueError when cas_id is None."""
    from modelado.ikam_graph_repository import store_fragment
    from ikam.fragments import Fragment

    cx, _ = _make_mock_connection()
    frag = Fragment(value="hello")  # no cas_id

    with pytest.raises(ValueError, match="cas_id"):
        store_fragment(cx, frag, project_id="proj1")


@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_store_fragment_default_ref_is_main(mock_require):
    """Default ref is the canonical main branch when not specified."""
    from modelado.ikam_graph_repository import store_fragment
    from ikam.forja.cas import cas_fragment

    cx, cursor = _make_mock_connection()
    frag = cas_fragment({"text": "default env"}, "text/ikam-paragraph")

    store_fragment(cx, frag, project_id="proj1")

    params = cursor.execute.call_args[0][1]
    assert params[1] == "refs/heads/main"


@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_store_fragment_with_operation_id(mock_require):
    """operation_id is passed through to the INSERT."""
    from modelado.ikam_graph_repository import store_fragment
    from ikam.forja.cas import cas_fragment

    cx, cursor = _make_mock_connection()
    frag = cas_fragment({"text": "with op"}, "text/ikam-paragraph")
    ref = "refs/heads/staging/review"

    store_fragment(cx, frag, project_id="proj1", ref=ref, operation_id="op_123")

    params = cursor.execute.call_args[0][1]
    assert params[2] == "op_123"  # operation_id
    assert params[1] == ref


@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_store_fragment_treats_same_cas_across_refs_as_membership(mock_require):
    """Same CAS can be attached to multiple refs without changing payload identity."""
    from modelado.ikam_graph_repository import store_fragment
    from ikam.forja.cas import cas_fragment

    cx, cursor = _make_mock_connection()
    frag = cas_fragment({"text": "same payload"}, "text/ikam-paragraph")

    store_fragment(cx, frag, project_id="proj1", ref="refs/heads/main")
    store_fragment(cx, frag, project_id="proj1", ref="refs/heads/run/run-123")

    assert cursor.execute.call_count == 2
    first_params = cursor.execute.call_args_list[0][0][1]
    second_params = cursor.execute.call_args_list[1][0][1]
    assert first_params[0] == second_params[0] == frag.cas_id
    assert first_params[1] == "refs/heads/main"
    assert second_params[1] == "refs/heads/run/run-123"
