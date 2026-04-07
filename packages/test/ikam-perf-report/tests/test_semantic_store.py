from ikam_perf_report.graph.semantic_store import SemanticGraphStore


def test_store_tracks_entities_and_relations():
    store = SemanticGraphStore()
    store.add_entity(
        {
            "id": "e1",
            "label": "Revenue growth",
            "kind": "claim",
            "payload": {},
            "confidence": 0.9,
            "evidence": ["model-inputs"],
            "referenced_context": ["q1"],
        }
    )
    store.add_relation(
        {
            "id": "r1",
            "kind": "supports",
            "source": "e1",
            "target": "e1",
            "payload": {},
            "confidence": 0.8,
            "evidence": ["story-draft"],
            "referenced_context": ["deck-1"],
        }
    )
    summary = store.summary()
    assert summary["entities"] == 1
    assert summary["relations"] == 1
