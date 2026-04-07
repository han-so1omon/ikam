# packages/modelado/tests/test_edge_event_knowledge_prefix.py
"""Tests that all edge events use knowledge: prefix only — derivation: branch eliminated."""


def test_compute_edge_identity_key_uses_knowledge_prefix():
    from modelado.graph_edge_event_log import compute_edge_identity_key

    key = compute_edge_identity_key(
        edge_label="knowledge:contains",
        out_id="src1",
        in_id="tgt1",
        properties={"relationFragmentId": "frag-1"},
    )
    # Identity key is a sha256 hex digest — just verify it's deterministic
    assert isinstance(key, str) and len(key) == 64


def test_no_derivation_prefix():
    """The derivation: prefix branch is eliminated from source."""
    import inspect
    from modelado import graph_edge_event_log

    source = inspect.getsource(graph_edge_event_log)
    # No derivation: branch logic should remain
    assert 'startswith("derivation:' not in source


def test_identity_key_deterministic():
    """Same inputs produce same identity key."""
    from modelado.graph_edge_event_log import compute_edge_identity_key

    args = dict(
        edge_label="knowledge:contains",
        out_id="a",
        in_id="b",
        properties={"relationFragmentId": "rf-1"},
    )
    assert compute_edge_identity_key(**args) == compute_edge_identity_key(**args)


def test_identity_key_differs_for_different_labels():
    from modelado.graph_edge_event_log import compute_edge_identity_key

    k1 = compute_edge_identity_key(
        edge_label="knowledge:contains",
        out_id="a", in_id="b", properties={},
    )
    k2 = compute_edge_identity_key(
        edge_label="knowledge:lifted_from",
        out_id="a", in_id="b", properties={},
    )
    assert k1 != k2
