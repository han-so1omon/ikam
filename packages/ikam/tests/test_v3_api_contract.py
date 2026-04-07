"""V3 API contract tests for base-api migration.

Verify that V3 fragment model boundary is respected and API response models
expose V3-compatible shapes.
"""

from __future__ import annotations

import importlib
from pathlib import Path
import sys

import pytest


ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/ikam/src"))
sys.path.insert(0, str(ROOT / "packages/modelado/src"))


# ---------------------------------------------------------------------------
# 1. V3 Fragment namespace boundary
# ---------------------------------------------------------------------------


def test_v3_fragments_exports_only_v3_types():
    """ikam.fragments must NOT export FragmentType, TextBlock, TextFragmentContent."""
    mod = importlib.import_module("ikam.fragments")
    assert hasattr(mod, "Fragment"), "V3 Fragment must be exported"
    assert hasattr(mod, "Relation"), "V3 Relation must be exported"
    assert hasattr(mod, "RELATION_MIME"), "RELATION_MIME must be exported"

    # Legacy symbols must NOT be present
    for legacy_name in ("FragmentType", "TextBlock", "TextFragmentContent",
                        "BinaryFragmentContent", "DataFragmentContent",
                        "DecompositionConfig", "ReconstructionConfig"):
        assert not hasattr(mod, legacy_name), (
            f"V3 ikam.fragments must not export legacy symbol '{legacy_name}'"
        )


def test_legacy_compat_module_removed():
    """Legacy adapter compat module is removed in V3-only codebase."""
    module_name = "modelado." + "_legacy_adapter_compat"
    with pytest.raises(ModuleNotFoundError):
        importlib.import_module(module_name)


# ---------------------------------------------------------------------------
# 2. V3 Fragment construction invariants
# ---------------------------------------------------------------------------


def test_v3_fragment_requires_cas_id_or_value():
    """V3 Fragment(cas_id=None, value=None) must raise ValueError (F1)."""
    from ikam.fragments import Fragment
    with pytest.raises(ValueError, match="F1"):
        Fragment(cas_id=None, value=None)


def test_v3_fragment_accepts_inline_value():
    """V3 Fragment with value only (CAS-only deferred) is valid."""
    from ikam.fragments import Fragment
    f = Fragment(value={"text": "hello"}, mime_type="text/plain")
    assert f.value == {"text": "hello"}
    assert f.mime_type == "text/plain"
    assert f.cas_id is None


def test_v3_fragment_accepts_cas_id_only():
    """V3 Fragment with cas_id only (value deferred/not-inline) is valid."""
    from ikam.fragments import Fragment
    f = Fragment(cas_id="abc123")
    assert f.cas_id == "abc123"
    assert f.value is None


# ---------------------------------------------------------------------------
# 3. Storage fragment API remains available in package-only repo
# ---------------------------------------------------------------------------


def test_storage_fragment_api_importable():
    """Package-only IKAM repo exposes StoredFragment and CAS helpers."""
    from ikam.graph import StoredFragment, _cas_hex

    assert StoredFragment is not None
    assert callable(_cas_hex)


def test_storage_fragment_fragment_name_removed_in_clean_break_mode():
    """ikam.graph.Fragment must not remain as a compatibility alias."""
    mod = importlib.import_module("ikam.graph")
    assert not hasattr(mod, "Fragment")


# ---------------------------------------------------------------------------
# 4. DomainFragmentResponse V3 compatibility
# ---------------------------------------------------------------------------


def test_domain_fragment_response_has_v3_fields():
    """DomainFragmentResponse response model must include mime_type field
    for V3 compatibility (MIME-based semantics replace FragmentType enum)."""
    # Import the response model from ikam_graph router
    try:
        from narraciones_base_api.app.api.ikam_graph import DomainFragmentResponse
    except ImportError:
        pytest.skip("narraciones_base_api not importable in this environment")

    fields = DomainFragmentResponse.model_fields
    assert "mime_type" in fields, "DomainFragmentResponse must have 'mime_type' field for V3"


def test_domain_fragment_response_accepts_v3_shape():
    """DomainFragmentResponse should accept data with V3 mime_type instead of
    relying on FragmentType enum in the 'type' field."""
    try:
        from narraciones_base_api.app.api.ikam_graph import DomainFragmentResponse
    except ImportError:
        pytest.skip("narraciones_base_api not importable in this environment")

    # V3-style response: mime_type is primary, type is for backward compat
    resp = DomainFragmentResponse(
        id="frag-001",
        artifact_id="art-001",
        level=0,
        type="text",  # legacy compat
        content={"summary": "test"},
        salience=0.5,
        created_at="2026-01-01T00:00:00Z",
        updated_at="2026-01-01T00:00:00Z",
        mime_type="text/plain",
    )
    assert resp.mime_type == "text/plain"


# ---------------------------------------------------------------------------
# 5. V3 adapter path exists for round-trip
# ---------------------------------------------------------------------------


def test_v3_adapter_path_exists():
    """V3 adapter functions must be importable from ikam.adapters."""
    from ikam.adapters import (
        v3_fragment_to_cas_bytes,
        v3_fragment_from_cas_bytes,
        v3_to_storage,
    )
    # Verify they are callable
    assert callable(v3_fragment_to_cas_bytes)
    assert callable(v3_fragment_from_cas_bytes)
    assert callable(v3_to_storage)


def test_v3_adapter_roundtrip():
    """V3 fragment → CAS bytes → V3 fragment round-trip is lossless."""
    from ikam.fragments import Fragment
    from ikam.adapters import v3_fragment_to_cas_bytes, v3_fragment_from_cas_bytes
    from ikam.graph import _cas_hex

    original = Fragment(value={"key": "val"}, mime_type="application/json")
    cas_bytes = v3_fragment_to_cas_bytes(original)
    cas_id = _cas_hex(cas_bytes)
    restored = v3_fragment_from_cas_bytes(cas_id=cas_id, payload=cas_bytes)

    assert restored.value == original.value
    assert restored.mime_type == original.mime_type
    assert restored.cas_id == cas_id


# ---------------------------------------------------------------------------
# 6. Provenance projection compatibility
# ---------------------------------------------------------------------------


def test_provenance_projection_importable():
    """LatestOutputProjection must be importable from modelado.provenance_views."""
    from modelado.provenance_views import LatestOutputProjection, rebuild_projection
    assert callable(rebuild_projection)
    proj = LatestOutputProjection()
    assert proj.latest_output("x", "y") is None
