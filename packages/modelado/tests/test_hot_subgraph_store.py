from modelado.hot_subgraph_store import InMemoryHotSubgraphStore


def test_put_returns_deterministic_subgraph_ref() -> None:
    store = InMemoryHotSubgraphStore()

    payload = {
        "nodes": [
            {"id": "b", "value": 2},
            {"id": "a", "value": 1},
        ],
        "meta": {"z": 1, "a": 2},
    }

    first = store.put(payload)
    second = store.put(payload)

    assert first == second
    assert first == {
        "type": "subgraph_ref",
        "head_fragment_id": first["head_fragment_id"],
    }
    assert first["head_fragment_id"].startswith("cas://sha256:")


def test_put_canonicalizes_equivalent_payloads() -> None:
    store = InMemoryHotSubgraphStore()

    first = store.put({"b": 2, "a": {"d": 4, "c": 3}})
    second = store.put({"a": {"c": 3, "d": 4}, "b": 2})

    assert second == first


def test_get_returns_canonical_copy_not_original_object() -> None:
    store = InMemoryHotSubgraphStore()
    payload = {"items": [{"id": "a"}], "meta": {"status": "hot"}}

    ref = store.put(payload)
    payload["items"][0]["id"] = "mutated"
    payload["meta"]["status"] = "changed"

    stored = store.get(ref)

    assert stored == {"items": [{"id": "a"}], "meta": {"status": "hot"}}
    assert stored is not payload


def test_get_returns_none_for_unknown_ref() -> None:
    store = InMemoryHotSubgraphStore()

    assert store.get(
        {
            "type": "subgraph_ref",
            "head_fragment_id": "cas://sha256:missing",
        }
    ) is None
