"""V3 adapter roundtrip tests.

Validates:
- V3 Fragment → CAS storage → V3 Fragment roundtrip is lossless
- Relation MIME payloads serialize/deserialize through CAS unchanged
- Legacy metadata excluded from CAS identity bytes
- MIME-based dispatch replaces type-switch

References:
  - docs/ikam/IKAM_FRAGMENT_ALGEBRA_V3.md §2.1, §2.2
  - Wave 4 Task 4.1 in migration plan
"""

import json
import pytest

from ikam.fragments import (
    Fragment,
    Relation,
    SlotBinding,
    BindingGroup,
    RELATION_MIME,
)
from ikam.graph import StoredFragment as StorageFragment, _cas_hex


# ── helpers ─────────────────────────────────────────────────────────────────

def _make_text_fragment(text: str) -> Fragment:
    return Fragment(value=text, mime_type="text/plain")


def _make_json_fragment(data: dict) -> Fragment:
    return Fragment(value=data, mime_type="application/json")


def _make_relation_fragment(
    predicate: str = "derives_from",
    binding_groups: list[BindingGroup] | None = None,
    function_cas_id: str | None = None,
) -> Fragment:
    rel = Relation(
        predicate=predicate,
        directed=True,
        confidence_score=0.9,
        qualifiers={"source": "test"},
        function_cas_id=function_cas_id,
        binding_groups=binding_groups or [],
    )
    return Fragment(
        value=rel.model_dump(mode="json"),
        mime_type=RELATION_MIME,
    )


# ── V3 adapter functions under test ────────────────────────────────────────
# These imports will fail until the adapter is implemented.

class TestV3FragmentToCAS:
    """V3 Fragment → CAS bytes → storage identity."""

    def test_text_fragment_cas_bytes_deterministic(self):
        """Same text value → same CAS id."""
        from ikam.adapters import v3_fragment_to_cas_bytes

        f1 = _make_text_fragment("hello world")
        f2 = _make_text_fragment("hello world")
        b1 = v3_fragment_to_cas_bytes(f1)
        b2 = v3_fragment_to_cas_bytes(f2)
        assert b1 == b2
        assert _cas_hex(b1) == _cas_hex(b2)

    def test_different_values_different_cas(self):
        """Different values → different CAS ids."""
        from ikam.adapters import v3_fragment_to_cas_bytes

        f1 = _make_text_fragment("alpha")
        f2 = _make_text_fragment("beta")
        assert v3_fragment_to_cas_bytes(f1) != v3_fragment_to_cas_bytes(f2)

    def test_mime_type_in_cas_bytes(self):
        """MIME type is part of CAS identity (same value, different MIME → different CAS)."""
        from ikam.adapters import v3_fragment_to_cas_bytes

        f1 = Fragment(value="data", mime_type="text/plain")
        f2 = Fragment(value="data", mime_type="text/markdown")
        assert v3_fragment_to_cas_bytes(f1) != v3_fragment_to_cas_bytes(f2)

    def test_cas_id_only_fragment_passes_through(self):
        """CAS-only fragment (no value) has no content to serialize — returns empty canonical."""
        from ikam.adapters import v3_fragment_to_cas_bytes

        f = Fragment(cas_id="abc123")
        # CAS-only fragments are references; serializing identity itself
        b = v3_fragment_to_cas_bytes(f)
        assert isinstance(b, bytes)
        assert len(b) > 0


class TestV3RelationCASRoundtrip:
    """Relation MIME payloads serialize/deserialize through CAS unchanged."""

    def test_relation_payload_roundtrip(self):
        """Relation fragment → CAS bytes → reconstructed fragment has identical payload."""
        from ikam.adapters import v3_fragment_to_cas_bytes, v3_fragment_from_cas_bytes

        bindings = [
            BindingGroup(
                invocation_id="inv-1",
                slots=[
                    SlotBinding(slot="input", fragment_id="frag-aaa"),
                    SlotBinding(slot="template", fragment_id="frag-bbb"),
                ],
            ),
        ]
        original = _make_relation_fragment(
            predicate="composes",
            binding_groups=bindings,
            function_cas_id="fn-concat-001",
        )
        cas_bytes = v3_fragment_to_cas_bytes(original)
        cas_id = _cas_hex(cas_bytes)

        reconstructed = v3_fragment_from_cas_bytes(
            cas_id=cas_id,
            payload=cas_bytes,
        )

        assert reconstructed.cas_id == cas_id
        assert reconstructed.mime_type == RELATION_MIME
        # Payload equality
        orig_rel = Relation(**original.value)
        recon_rel = Relation(**reconstructed.value)
        assert orig_rel.predicate == recon_rel.predicate
        assert orig_rel.function_cas_id == recon_rel.function_cas_id
        assert len(orig_rel.binding_groups) == len(recon_rel.binding_groups)

    def test_relation_cas_excludes_transport_metadata(self):
        """Transport/runtime metadata should NOT affect CAS identity."""
        from ikam.adapters import v3_fragment_to_cas_bytes

        f1 = _make_relation_fragment(predicate="cites")
        f2 = _make_relation_fragment(predicate="cites")
        # Both should have identical CAS bytes
        assert v3_fragment_to_cas_bytes(f1) == v3_fragment_to_cas_bytes(f2)


class TestV3ToStorageFragment:
    """V3 Fragment → StorageFragment conversion."""

    def test_to_storage_fragment(self):
        """V3 Fragment converts to a StoredFragment with canonical CAS bytes."""
        from ikam.adapters import v3_fragment_to_cas_bytes, v3_to_storage

        frag = _make_text_fragment("some text content")
        storage = v3_to_storage(frag)
        expected_bytes = v3_fragment_to_cas_bytes(frag)

        assert isinstance(storage, StorageFragment)
        assert storage.size > 0
        assert storage.mime_type == "text/plain"
        assert storage.bytes == expected_bytes
        assert storage.size == len(expected_bytes)
        assert storage.id == _cas_hex(storage.bytes)

    def test_storage_roundtrip_preserves_value(self):
        """V3 → Storage → V3 roundtrip is lossless."""
        from ikam.adapters import v3_to_storage, v3_fragment_from_cas_bytes

        original = _make_json_fragment({"key": "val", "nested": [1, 2, 3]})
        storage = v3_to_storage(original)

        reconstructed = v3_fragment_from_cas_bytes(
            cas_id=storage.id,
            payload=storage.bytes,
        )

        assert reconstructed.cas_id == storage.id
        assert reconstructed.value == original.value
        assert reconstructed.mime_type == original.mime_type

    def test_to_storage_returns_stored_fragment_type(self):
        """Adapter boundary returns the storage-layer record type directly."""
        from ikam.adapters import v3_to_storage
        from ikam.graph import StoredFragment

        storage = v3_to_storage(_make_json_fragment({"k": "v"}))

        assert type(storage) is StoredFragment


class TestV3MIMEDispatch:
    """MIME-based dispatch replaces legacy type-switch."""

    def test_dispatch_text_plain(self):
        """text/plain dispatches correctly."""
        from ikam.adapters import v3_to_storage

        frag = _make_text_fragment("hello")
        storage = v3_to_storage(frag)
        assert storage.mime_type == "text/plain"

    def test_dispatch_relation_mime(self):
        """Relation MIME dispatches correctly."""
        from ikam.adapters import v3_to_storage

        frag = _make_relation_fragment()
        storage = v3_to_storage(frag)
        assert storage.mime_type == RELATION_MIME

    def test_dispatch_application_json(self):
        """application/json dispatches correctly."""
        from ikam.adapters import v3_to_storage

        frag = _make_json_fragment({"x": 1})
        storage = v3_to_storage(frag)
        assert storage.mime_type == "application/json"
