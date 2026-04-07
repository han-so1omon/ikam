"""Tests for edge event emission using knowledge: prefix.

Validates that emit_edge_event() delegates to append_graph_edge_event
with the correct knowledge:<predicate> format.
"""
import pytest
from unittest.mock import MagicMock, patch


@patch("modelado.ikam_graph_repository.append_graph_edge_event")
@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_emit_edge_event_uses_knowledge_prefix(mock_require, mock_append):
    """Edge label is formatted as knowledge:<predicate>."""
    from modelado.ikam_graph_repository import emit_edge_event

    cx = MagicMock()

    emit_edge_event(
        cx,
        source_id="frag_abc",
        target_id="frag_def",
        predicate="contains",
        project_id="proj1",
    )

    mock_append.assert_called_once()
    kwargs = mock_append.call_args
    assert kwargs[1]["edge_label"] == "knowledge:contains"
    assert kwargs[1]["out_id"] == "frag_abc"
    assert kwargs[1]["in_id"] == "frag_def"
    assert kwargs[1]["project_id"] == "proj1"
    assert kwargs[1]["op"] == "upsert"


@patch("modelado.ikam_graph_repository.append_graph_edge_event")
@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_emit_edge_event_passes_properties(mock_require, mock_append):
    """Custom properties are forwarded to append_graph_edge_event."""
    from modelado.ikam_graph_repository import emit_edge_event

    cx = MagicMock()

    emit_edge_event(
        cx,
        source_id="a",
        target_id="b",
        predicate="derives",
        project_id="proj1",
        properties={"source": "upload", "weight": 0.9},
    )

    kwargs = mock_append.call_args[1]
    assert kwargs["properties"]["source"] == "upload"
    assert kwargs["properties"]["weight"] == 0.9


@patch("modelado.ikam_graph_repository.append_graph_edge_event")
@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_emit_edge_event_empty_properties(mock_require, mock_append):
    """When no properties given, scope qualifiers are injected."""
    from modelado.ikam_graph_repository import emit_edge_event

    cx = MagicMock()

    emit_edge_event(
        cx,
        source_id="a",
        target_id="b",
        predicate="contains",
        project_id="proj1",
    )

    kwargs = mock_append.call_args[1]
    assert kwargs["properties"]["envType"] == "committed"
    assert kwargs["properties"]["envId"] == "main"
