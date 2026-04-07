from ikam.adapters import compose_manifests, empty_manifest


def _make_manifest(kind: str, fragments: list[dict]) -> dict:
    return {
        "schemaVersion": 1,
        "artifactId": "test-artifact",
        "kind": kind,
        "fragments": fragments,
    }


def test_left_identity():
    """ε ⊕ m = m"""
    m = _make_manifest("document", [
        {"fragmentId": "f1", "parentFragmentId": None, "level": 0, "type": "section"},
    ])
    result = compose_manifests(empty_manifest("document"), m)
    assert result["fragments"] == m["fragments"]


def test_right_identity():
    """m ⊕ ε = m"""
    m = _make_manifest("document", [
        {"fragmentId": "f1", "parentFragmentId": None, "level": 0, "type": "section"},
    ])
    result = compose_manifests(m, empty_manifest("document"))
    assert result["fragments"] == m["fragments"]


def test_associativity():
    """(a ⊕ b) ⊕ c = a ⊕ (b ⊕ c)"""
    a = _make_manifest("document", [
        {"fragmentId": "f1", "parentFragmentId": None, "level": 0, "type": "heading"},
    ])
    b = _make_manifest("document", [
        {"fragmentId": "f2", "parentFragmentId": None, "level": 0, "type": "paragraph"},
    ])
    c = _make_manifest("document", [
        {"fragmentId": "f3", "parentFragmentId": None, "level": 0, "type": "table"},
    ])
    left = compose_manifests(compose_manifests(a, b), c)
    right = compose_manifests(a, compose_manifests(b, c))
    assert left["fragments"] == right["fragments"]


def test_compose_rejects_incompatible_kinds():
    """⊕ requires compatible kind fields."""
    m1 = _make_manifest("document", [{"fragmentId": "f1", "parentFragmentId": None, "level": 0, "type": "section"}])
    m2 = _make_manifest("spreadsheet", [{"fragmentId": "f2", "parentFragmentId": None, "level": 0, "type": "cell"}])
    import pytest
    with pytest.raises(ValueError, match="kind"):
        compose_manifests(m1, m2)


def test_empty_manifest_has_no_fragments():
    """ε has empty fragment list."""
    e = empty_manifest("document")
    assert e["fragments"] == []
    assert e["kind"] == "document"
    assert e["schemaVersion"] == 1
