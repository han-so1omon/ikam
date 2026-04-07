"""Tests for public cas_fragment() function."""

from ikam.fragments import Fragment


def test_cas_fragment_returns_fragment_with_cas_id():
    from ikam.forja.cas import cas_fragment
    frag = cas_fragment({"text": "hello"}, "text/ikam-paragraph")
    assert isinstance(frag, Fragment)
    assert frag.cas_id is not None
    assert len(frag.cas_id) > 0
    assert frag.mime_type == "text/ikam-paragraph"
    assert frag.value == {"text": "hello"}


def test_cas_fragment_deterministic():
    from ikam.forja.cas import cas_fragment
    f1 = cas_fragment({"x": 1}, "application/json")
    f2 = cas_fragment({"x": 1}, "application/json")
    assert f1.cas_id == f2.cas_id


def test_cas_fragment_different_values_different_ids():
    from ikam.forja.cas import cas_fragment
    f1 = cas_fragment({"x": 1}, "application/json")
    f2 = cas_fragment({"x": 2}, "application/json")
    assert f1.cas_id != f2.cas_id


def test_cas_fragment_importable_from_forja():
    from ikam.forja import cas_fragment
    assert callable(cas_fragment)


def test_cas_fragment_different_mime_different_ids():
    """Same value with different MIME types should produce different CAS IDs,
    since MIME type is part of the canonical representation."""
    from ikam.forja.cas import cas_fragment
    f1 = cas_fragment({"x": 1}, "text/plain")
    f2 = cas_fragment({"x": 1}, "application/json")
    assert f1.cas_id != f2.cas_id


def test_cas_fragment_matches_adapter_canonical_bytes_and_cas_id():
    from ikam.adapters import v3_fragment_to_cas_bytes
    from ikam.forja.cas import cas_fragment
    from ikam.graph import _cas_hex

    value = {"x": 1, "nested": ["a", 2]}
    mime_type = "application/json"

    fragment = cas_fragment(value, mime_type)
    expected_bytes = v3_fragment_to_cas_bytes(Fragment(value=value, mime_type=mime_type))

    assert fragment.cas_id == _cas_hex(expected_bytes)


def test_cas_fragment_is_adapter_wrapper_equivalent_to_v3_to_storage():
    from ikam.adapters import v3_to_storage
    from ikam.forja.cas import cas_fragment

    value = {"text": "hello"}
    mime_type = "text/ikam-paragraph"

    fragment = cas_fragment(value, mime_type)
    storage = v3_to_storage(Fragment(value=value, mime_type=mime_type))

    assert fragment == Fragment(cas_id=storage.id, value=value, mime_type=mime_type)


def test_cas_fragment_delegates_storage_conversion_to_adapters(monkeypatch):
    import ikam.adapters as adapters
    from ikam.forja.cas import cas_fragment
    from ikam.graph import StoredFragment

    calls: list[Fragment] = []

    def fake_v3_to_storage(fragment: Fragment) -> StoredFragment:
        calls.append(fragment)
        return StoredFragment(
            id="adapter-derived-id",
            bytes=b'{"from":"adapter"}',
            mime_type=fragment.mime_type or "application/octet-stream",
            size=len(b'{"from":"adapter"}'),
        )

    monkeypatch.setattr(adapters, "v3_to_storage", fake_v3_to_storage)

    fragment = cas_fragment({"text": "wrapped"}, "text/ikam-paragraph")

    assert calls == [Fragment(value={"text": "wrapped"}, mime_type="text/ikam-paragraph")]
    assert fragment == Fragment(
        cas_id="adapter-derived-id",
        value={"text": "wrapped"},
        mime_type="text/ikam-paragraph",
    )
