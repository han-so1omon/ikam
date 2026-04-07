"""Tests for format composition strategy."""


def test_format_applies_numeric_format():
    from ikam.forja.composers.format_strategy import format_compose
    from ikam.fragments import Fragment

    raw = Fragment(value={"raw_value": 1234.56}, mime_type="application/ikam-value-cell+json")
    fmt = Fragment(value={"format": "currency:usd:2dp"}, mime_type="application/ikam-style+json")
    result = format_compose(raw, fmt)
    assert "formatted" in result.value
    assert result.value["raw_value"] == 1234.56


def test_format_currency_output():
    from ikam.forja.composers.format_strategy import format_compose
    from ikam.fragments import Fragment

    raw = Fragment(value={"raw_value": 1234.56}, mime_type="application/ikam-value-cell+json")
    fmt = Fragment(value={"format": "currency:usd:2dp"}, mime_type="application/ikam-style+json")
    result = format_compose(raw, fmt)
    assert result.value["formatted"] == "$1,234.56"
    assert result.value["format_spec"] == "currency:usd:2dp"


def test_format_preserves_cas():
    from ikam.forja.composers.format_strategy import format_compose
    from ikam.fragments import Fragment

    raw = Fragment(value={"raw_value": 100.0}, mime_type="application/ikam-value-cell+json")
    fmt = Fragment(value={"format": "currency:usd:0dp"}, mime_type="application/ikam-style+json")
    r1 = format_compose(raw, fmt)
    r2 = format_compose(raw, fmt)
    assert r1.cas_id == r2.cas_id  # Deterministic


def test_format_no_spec_falls_back_to_str():
    from ikam.forja.composers.format_strategy import format_compose
    from ikam.fragments import Fragment

    raw = Fragment(value={"raw_value": 42}, mime_type="application/ikam-value-cell+json")
    fmt = Fragment(value={}, mime_type="application/ikam-style+json")
    result = format_compose(raw, fmt)
    assert result.value["formatted"] == "42"
    assert result.value["format_spec"] == ""
