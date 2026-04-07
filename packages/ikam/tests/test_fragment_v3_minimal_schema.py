"""Tests for V3 minimal fragment schema.

Covers conformance requirement V3-FRAG-1:
- graph-id-only fragment valid in the current runtime model
- CAS-only fragment valid
- inline-only fragment valid
- dual-state fragment shape valid
- At least one of fragment_id, cas_id, or value must be present (F1)
- Boundary docs reflect runtime Fragment vs storage StoredFragment naming
"""
from __future__ import annotations

import json
from pathlib import Path
import pytest
from pydantic import ValidationError


def test_cas_only_fragment_valid():
    """A fragment with only cas_id set is valid."""
    from ikam.fragments import Fragment

    frag = Fragment(cas_id="abc123deadbeef", mime_type="text/plain")
    assert frag.cas_id == "abc123deadbeef"
    assert frag.value is None
    assert frag.mime_type == "text/plain"


def test_inline_only_fragment_valid():
    """A fragment with only value set is valid."""
    from ikam.fragments import Fragment

    frag = Fragment(value={"summary": "hello"}, mime_type="application/json")
    assert frag.value == {"summary": "hello"}
    assert frag.cas_id is None


def test_dual_state_fragment_valid():
    """A fragment with both cas_id and value is valid in the current runtime model."""
    from ikam.fragments import Fragment

    frag = Fragment(
        cas_id="abc123",
        value={"data": [1, 2, 3]},
        mime_type="application/json",
    )
    assert frag.cas_id == "abc123"
    assert frag.value == {"data": [1, 2, 3]}


def test_empty_fragment_rejected():
    """A fragment with no fragment_id, cas_id, or value is invalid (invariant F1)."""
    from ikam.fragments import Fragment

    with pytest.raises((ValidationError, ValueError)):
        Fragment(mime_type="text/plain")


def test_fragment_has_no_legacy_fields():
    """V3 Fragment should not have legacy fields like artifact_id, level, type, radical_refs."""
    from ikam.fragments import Fragment

    frag = Fragment(cas_id="abc123", mime_type="text/plain")
    # V3 Fragment keeps one public runtime Fragment plus graph-scoped fragment_id.
    assert not hasattr(frag, "artifact_id") or "artifact_id" not in Fragment.model_fields
    assert not hasattr(frag, "level") or "level" not in Fragment.model_fields
    assert not hasattr(frag, "radical_refs") or "radical_refs" not in Fragment.model_fields
    assert not hasattr(frag, "salience") or "salience" not in Fragment.model_fields


def test_fragment_model_fields_minimal():
    """V3 Fragment has the current runtime field set."""
    from ikam.fragments import Fragment

    field_names = set(Fragment.model_fields.keys())
    expected = {"fragment_id", "cas_id", "value", "mime_type"}
    assert field_names == expected, f"Got fields: {field_names}, expected: {expected}"


def test_fragment_id_only_fragment_valid():
    """A fragment with only fragment_id set is valid in the current runtime model."""
    from ikam.fragments import Fragment

    frag = Fragment(fragment_id="frag-123")
    assert frag.fragment_id == "frag-123"
    assert frag.cas_id is None
    assert frag.value is None


def test_fragment_id_is_not_a_storage_alias():
    """Storage-layer naming is explicit and separate from runtime Fragment."""
    graph_py = Path(__file__).resolve().parents[1] / "src" / "ikam" / "graph.py"
    text = graph_py.read_text()

    assert "class StoredFragment(BaseModel):" in text
    assert "class Fragment(BaseModel):" not in text


def test_v3_doc_mentions_current_fragment_shape_and_storage_name():
    """The V3 doc should match the current runtime fragment boundary."""
    doc = Path(__file__).resolve().parents[3] / "docs" / "ikam" / "IKAM_FRAGMENT_ALGEBRA_V3.md"
    text = doc.read_text()

    assert "fragment_id: Optional[str] = None" in text
    assert "StoredFragment" in text


def test_scratch_doc_marks_boundary_decision_as_final():
    """The scratch note should reflect the implemented clean-break decision."""
    doc = Path(__file__).resolve().parents[3] / "docs" / "scratch" / "ikam-graph-runtime-and-fragment-cleanup.md"
    text = doc.read_text()

    assert "`ikam.graph.StoredFragment`" in text
    assert "This is now the implemented boundary" in text
    assert "This is not a final decision yet" not in text
