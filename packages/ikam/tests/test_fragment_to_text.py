"""Tests for fragment_to_text conversion."""


def test_text_fragment():
    from ikam.ir.text_conversion import fragment_to_text
    from ikam.fragments import Fragment
    frag = Fragment(value={"text": "Revenue Growth"}, mime_type="text/ikam-heading")
    result = fragment_to_text(frag)
    assert "Revenue Growth" in result
    assert "[text/ikam-heading]" in result


def test_json_value_fragment():
    from ikam.ir.text_conversion import fragment_to_text
    from ikam.fragments import Fragment
    frag = Fragment(
        value={"formula": "=SUM(B2:B10)", "result": 42500},
        mime_type="application/ikam-formula-cell+json",
    )
    result = fragment_to_text(frag)
    assert "SUM(B2:B10)" in result
    assert "[application/ikam-formula-cell+json]" in result


def test_bytes_only_fragment():
    from ikam.ir.text_conversion import fragment_to_text
    from ikam.fragments import Fragment
    import base64
    frag = Fragment(
        value={"bytes_b64": base64.b64encode(b"hello").decode()},
        mime_type="application/octet-stream",
    )
    result = fragment_to_text(frag)
    assert "[application/octet-stream]" in result


def test_none_value():
    from ikam.ir.text_conversion import fragment_to_text
    from ikam.fragments import Fragment
    frag = Fragment(cas_id="abc123", mime_type="text/plain")
    result = fragment_to_text(frag)
    assert "[text/plain]" in result
    assert "abc123" in result
