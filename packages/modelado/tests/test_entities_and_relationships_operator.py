from __future__ import annotations

from modelado.environment_scope import EnvironmentScope
from modelado.operators.core import OperatorEnv, OperatorParams
from modelado.operators.registry import create_default_operator_registry


_DEV_SCOPE = EnvironmentScope(ref="refs/heads/run/test")


def test_entities_and_relationships_operator_returns_entity_relationship_rows_with_provenance() -> None:
    from modelado.operators.entities_and_relationships import EntitiesAndRelationshipsOperator

    operator = EntitiesAndRelationshipsOperator()
    env = OperatorEnv(seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)
    params = OperatorParams(
        name="parse.entities_and_relationships",
        parameters={
            "chunk_extraction_set": {
                "kind": "chunk_extraction_set",
                "source_subgraph_ref": "subgraph://run-1-document-set-load-documents",
                "subgraph_ref": "subgraph://run-1-chunk-extraction-set-parse-chunk",
                "extraction_refs": ["frag-chunk-1", "frag-chunk-2"],
                "chunk_extractions": [
                    {
                        "cas_id": "frag-chunk-1",
                        "mime_type": "application/vnd.ikam.chunk+json",
                        "value": {
                            "chunk_id": "doc-1:chunk:0",
                            "document_id": "doc-1",
                            "artifact_id": "artifact://doc-1",
                            "filename": "doc-1.md",
                            "text": "Alice founded Acme in Bogota.",
                            "span": {"start": 0, "end": 30},
                            "order": 0,
                        },
                    },
                    {
                        "cas_id": "frag-chunk-2",
                        "mime_type": "application/vnd.ikam.chunk+json",
                        "value": {
                            "chunk_id": "doc-1:chunk:1",
                            "document_id": "doc-1",
                            "artifact_id": "artifact://doc-1",
                            "filename": "doc-1.md",
                            "text": "Bob joined Acme later.",
                            "span": {"start": 31, "end": 53},
                            "order": 1,
                        },
                    },
                ],
            }
        },
    )

    result = operator.apply(None, params, env)

    assert result["status"] == "ok"
    assert result["source_kind"] == "chunk_extraction_set"
    assert result["summary"] == {
        "chunk_count": 2,
        "entity_relationship_fragment_count": 2,
        "entity_count": 4,
        "relationship_count": 4,
    }
    assert len(result["fragment_ids"]) == 2
    assert result["entity_relationship_set"]["kind"] == "entity_relationship_set"
    assert result["entity_relationship_set"]["source_subgraph_ref"] == "subgraph://run-1-chunk-extraction-set-parse-chunk"
    assert result["entity_relationship_set"]["subgraph_ref"] == "subgraph://run-1-chunk-extraction-set-parse-chunk-entities"
    assert result["entity_relationship_set"]["entity_relationship_refs"] == result["fragment_ids"]
    assert [row["chunk_id"] for row in result["entity_relationships"]] == [
        "doc-1:chunk:0",
        "doc-1:chunk:1",
    ]
    assert result["entity_relationships"][0]["document_id"] == "doc-1"
    assert result["entity_relationships"][0]["artifact_id"] == "artifact://doc-1"
    assert result["entity_relationships"][0]["chunk_fragment_id"] == "frag-chunk-1"
    assert result["entity_relationships"][0]["entities"] == [
        {"name": "Alice", "type": "candidate_entity"},
        {"name": "Acme", "type": "candidate_entity"},
        {"name": "Bogota", "type": "candidate_entity"},
    ]
    assert result["entity_relationships"][0]["relationships"] == [
        {"source": "Alice", "target": "Acme", "relationship": "co_occurs_in_chunk"},
        {"source": "Alice", "target": "Bogota", "relationship": "co_occurs_in_chunk"},
        {"source": "Acme", "target": "Bogota", "relationship": "co_occurs_in_chunk"},
    ]
    assert result["entity_relationships"][1]["relationships"] == [
        {"source": "Bob", "target": "Acme", "relationship": "co_occurs_in_chunk"}
    ]
    assert result["fragment_artifact_map"][result["fragment_ids"][0]] == "artifact://doc-1"


def test_entities_and_relationships_operator_registry_registers_operator(monkeypatch) -> None:
    from modelado.operators import EntitiesAndRelationshipsOperator

    class _FakeRegistry:
        def __init__(self, _cx: object, _manager: object, namespace: str) -> None:
            self.namespace = namespace
            self.entries: dict[str, object] = {}

        def list_keys(self) -> list[str]:
            return sorted(self.entries)

        def register(self, key: str, entry: object) -> None:
            self.entries[key] = entry

        def get(self, key: str) -> object | None:
            return self.entries.get(key)

    monkeypatch.setattr("modelado.operators.registry.OperatorRegistryAdapter", _FakeRegistry)

    registry = create_default_operator_registry(object(), object(), namespace="operators.default.test")

    assert isinstance(
        registry.get("modelado/operators/entities_and_relationships"),
        EntitiesAndRelationshipsOperator,
    )
