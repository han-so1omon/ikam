"""Tests for fragment ref promotion plans."""
import pytest
from unittest.mock import MagicMock, patch


@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_promote_fragment_moves_selected_fragment_ids_between_refs(mock_require):
    from modelado.ikam_graph_repository import promote_fragment

    cx = MagicMock()
    cursor = MagicMock()
    cx.cursor.return_value.__enter__ = MagicMock(return_value=cursor)
    cx.cursor.return_value.__exit__ = MagicMock(return_value=False)

    promote_fragment(
        cx,
        ["cas_abc", "cas_xyz"],
        source_ref="refs/heads/run/run-123",
        target_ref="refs/heads/main",
    )

    cursor.execute.assert_called_once()
    sql = cursor.execute.call_args[0][0]
    params = cursor.execute.call_args[0][1]

    assert "UPDATE ikam_fragment_store" in sql
    assert "SET ref = %s" in sql
    assert "cas_id = ANY(%s)" in sql
    assert "ref = %s" in sql
    assert params == ("refs/heads/main", ["cas_abc", "cas_xyz"], "refs/heads/run/run-123")


@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_promote_fragment_requires_explicit_source_and_target_refs(mock_require):
    from modelado.ikam_graph_repository import promote_fragment

    cx = MagicMock()

    with pytest.raises(TypeError):
        promote_fragment(cx, ["cas_xyz"], target_ref="refs/heads/main")

    with pytest.raises(TypeError):
        promote_fragment(cx, ["cas_xyz"], source_ref="refs/heads/run/run-123")

    with pytest.raises(TypeError):
        promote_fragment(cx, ["cas_xyz"], from_env="dev", to_env="committed")


@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_promote_fragment_rejects_empty_fragment_selection(mock_require):
    from modelado.ikam_graph_repository import promote_fragment

    cx = MagicMock()

    with pytest.raises(ValueError, match="fragment_ids must be non-empty"):
        promote_fragment(
            cx,
            [],
            source_ref="refs/heads/run/run-123",
            target_ref="refs/heads/main",
        )


@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_promote_fragment_rejects_same_source_and_target_ref(mock_require):
    from modelado.ikam_graph_repository import promote_fragment

    cx = MagicMock()

    with pytest.raises(ValueError, match="source_ref and target_ref must differ"):
        promote_fragment(
            cx,
            ["cas_abc"],
            source_ref="refs/heads/main",
            target_ref="refs/heads/main",
        )
