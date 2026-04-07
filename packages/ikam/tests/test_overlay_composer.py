"""Tests for overlay composition strategy."""


def test_overlay_applies_delta():
    from ikam.forja.composers.overlay import overlay_compose
    from ikam.fragments import Fragment

    base = Fragment(
        value={"text": "Original content", "key": "A"},
        mime_type="text/ikam-paragraph",
    )
    delta = Fragment(
        value={"key": "B", "extra": "added"},
        mime_type="text/ikam-paragraph",
    )
    result = overlay_compose(base, delta)
    assert result.value["text"] == "Original content"  # unchanged
    assert result.value["key"] == "B"  # overridden
    assert result.value["extra"] == "added"  # added


def test_overlay_empty_delta():
    from ikam.forja.composers.overlay import overlay_compose
    from ikam.fragments import Fragment

    base = Fragment(value={"text": "hello"}, mime_type="text/ikam-paragraph")
    delta = Fragment(value={}, mime_type="text/ikam-paragraph")
    result = overlay_compose(base, delta)
    assert result.value["text"] == "hello"


def test_overlay_has_cas_id():
    """Result fragment has a CAS ID from cas_fragment."""
    from ikam.forja.composers.overlay import overlay_compose
    from ikam.fragments import Fragment

    base = Fragment(value={"a": 1}, mime_type="text/ikam-paragraph")
    delta = Fragment(value={"b": 2}, mime_type="text/ikam-paragraph")
    result = overlay_compose(base, delta)
    assert result.cas_id is not None
    assert len(result.cas_id) > 0


def test_overlay_deterministic():
    """Same inputs produce same CAS ID."""
    from ikam.forja.composers.overlay import overlay_compose
    from ikam.fragments import Fragment

    base = Fragment(value={"x": 1}, mime_type="text/ikam-paragraph")
    delta = Fragment(value={"y": 2}, mime_type="text/ikam-paragraph")
    r1 = overlay_compose(base, delta)
    r2 = overlay_compose(base, delta)
    assert r1.cas_id == r2.cas_id
