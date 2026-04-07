"""Tests for concatenate composition strategy and strategy dispatcher."""
import base64


def test_concatenate_returns_bytes_in_order():
    """Concatenating fragment bytes_b64 in ID order reproduces original bytes."""
    from ikam.forja.composers.concatenate import concatenate_compose
    from ikam.forja.cas import cas_fragment

    chunk_a = b"# Heading\n\n"
    chunk_b = b"Paragraph text.\n"
    chunk_c = b"## Another heading\n"

    frag_a = cas_fragment(
        {"text": "# Heading", "index": 0, "bytes_b64": base64.b64encode(chunk_a).decode()},
        "text/ikam-heading",
    )
    frag_b = cas_fragment(
        {"text": "Paragraph text.", "index": 1, "bytes_b64": base64.b64encode(chunk_b).decode()},
        "text/ikam-paragraph",
    )
    frag_c = cas_fragment(
        {"text": "## Another heading", "index": 2, "bytes_b64": base64.b64encode(chunk_c).decode()},
        "text/ikam-heading",
    )

    fragment_store = {
        frag_a.cas_id: frag_a,
        frag_b.cas_id: frag_b,
        frag_c.cas_id: frag_c,
    }

    result = concatenate_compose(
        [frag_a.cas_id, frag_b.cas_id, frag_c.cas_id],
        fragment_store,
    )
    assert result == chunk_a + chunk_b + chunk_c


def test_concatenate_single_fragment():
    """Single fragment concatenation returns that fragment's bytes."""
    from ikam.forja.composers.concatenate import concatenate_compose
    from ikam.forja.cas import cas_fragment

    chunk = b"Only chunk"
    frag = cas_fragment(
        {"text": "Only chunk", "index": 0, "bytes_b64": base64.b64encode(chunk).decode()},
        "text/ikam-paragraph",
    )
    store = {frag.cas_id: frag}
    assert concatenate_compose([frag.cas_id], store) == chunk


def test_concatenate_empty_list_returns_empty_bytes():
    """Empty fragment list produces empty bytes."""
    from ikam.forja.composers.concatenate import concatenate_compose

    assert concatenate_compose([], {}) == b""


def test_concatenate_missing_fragment_raises():
    """Referencing a CAS ID not in store raises KeyError."""
    from ikam.forja.composers.concatenate import concatenate_compose

    try:
        concatenate_compose(["nonexistent_cas_id"], {})
        assert False, "Should have raised KeyError"
    except KeyError:
        pass


# ── Dispatcher tests ──


def test_dispatch_overlay():
    from ikam.forja.composers import dispatch_strategy
    from ikam.forja.composers.overlay import overlay_compose

    assert dispatch_strategy("overlay") is overlay_compose


def test_dispatch_concatenate():
    from ikam.forja.composers import dispatch_strategy
    from ikam.forja.composers.concatenate import concatenate_compose

    assert dispatch_strategy("concatenate") is concatenate_compose


def test_dispatch_format():
    from ikam.forja.composers import dispatch_strategy
    from ikam.forja.composers.format_strategy import format_compose

    assert dispatch_strategy("format") is format_compose


def test_dispatch_unknown_raises():
    from ikam.forja.composers import dispatch_strategy

    try:
        dispatch_strategy("nonexistent_strategy")
        assert False, "Should have raised ValueError"
    except ValueError as e:
        assert "nonexistent_strategy" in str(e)
