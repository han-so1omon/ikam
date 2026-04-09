from __future__ import annotations

from modelado.environment_scope import EnvironmentScope
from modelado.operators.core import OperatorEnv, OperatorParams
from modelado.operators.registry import create_default_operator_registry


_DEV_SCOPE = EnvironmentScope(ref="refs/heads/run/test")


def test_claims_operator_returns_claim_rows_with_entity_relationship_provenance() -> None:
    from modelado.operators.claims import ClaimsOperator

    operator = ClaimsOperator()
    env = OperatorEnv(seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)
    params = OperatorParams(
        name="map.conceptual.lift.claims",
        parameters={
            "entity_relationship_set": {
                "kind": "entity_relationship_set",
                "source_subgraph_ref": "subgraph://run-1-chunk-extraction-set-parse-chunk",
                "subgraph_ref": "subgraph://run-1-entity-relationship-set-parse-entities",
                "entity_relationship_refs": ["frag-ir-1", "frag-ir-2"],
                "entity_relationships": [
                    {
                        "cas_id": "frag-ir-1",
                        "mime_type": "application/vnd.ikam.entity-relationship+json",
                        "value": {
                            "fragment_id": "frag-er-row-1",
                            "chunk_fragment_id": "frag-chunk-1",
                            "chunk_id": "doc-1:chunk:0",
                            "document_id": "doc-1",
                            "artifact_id": "artifact://doc-1",
                            "filename": "doc-1.md",
                            "span": {"start": 0, "end": 30},
                            "order": 0,
                            "text": "Alice founded Acme in Bogota.",
                            "entities": [
                                {"name": "Alice", "type": "candidate_entity"},
                                {"name": "Acme", "type": "candidate_entity"},
                                {"name": "Bogota", "type": "candidate_entity"},
                            ],
                            "relationships": [
                                {"source": "Alice", "target": "Acme", "relationship": "co_occurs_in_chunk"},
                                {"source": "Acme", "target": "Bogota", "relationship": "co_occurs_in_chunk"},
                            ],
                        },
                    },
                    {
                        "cas_id": "frag-ir-2",
                        "mime_type": "application/vnd.ikam.entity-relationship+json",
                        "value": {
                            "fragment_id": "frag-er-row-2",
                            "chunk_fragment_id": "frag-chunk-2",
                            "chunk_id": "doc-1:chunk:1",
                            "document_id": "doc-1",
                            "artifact_id": "artifact://doc-1",
                            "filename": "doc-1.md",
                            "span": {"start": 31, "end": 53},
                            "order": 1,
                            "text": "Bob joined Acme later.",
                            "entities": [
                                {"name": "Bob", "type": "candidate_entity"},
                                {"name": "Acme", "type": "candidate_entity"},
                            ],
                            "relationships": [
                                {"source": "Bob", "target": "Acme", "relationship": "co_occurs_in_chunk"},
                            ],
                        },
                    },
                ],
            }
        },
    )

    result = operator.apply(None, params, env)

    assert result["status"] == "ok"
    assert result["source_kind"] == "entity_relationship_set"
    assert result["summary"] == {
        "entity_relationship_count": 2,
        "claim_fragment_count": 3,
        "claim_count": 3,
    }
    assert len(result["fragment_ids"]) == 3
    assert result["claim_set"] == {
        "kind": "claim_set",
        "source_subgraph_ref": "subgraph://run-1-chunk-extraction-set-parse-chunk",
        "subgraph_ref": "subgraph://run-1-chunk-extraction-set-parse-chunk-claims",
        "claim_refs": result["fragment_ids"],
        "claims": result["claim_set"]["claims"],
    }
    assert [claim["subject"] for claim in result["claims"]] == ["Alice", "Acme", "Bob"]
    assert [claim["object"] for claim in result["claims"]] == ["Acme", "Bogota", "Acme"]
    assert [claim["predicate"] for claim in result["claims"]] == [
        "co_occurs_in_chunk",
        "co_occurs_in_chunk",
        "co_occurs_in_chunk",
    ]
    assert result["claims"][0]["claim"] == "Alice co_occurs_in_chunk Acme"
    assert result["claims"][0]["entity_relationship_fragment_id"] == "frag-ir-1"
    assert result["claims"][0]["chunk_fragment_id"] == "frag-chunk-1"
    assert result["claims"][0]["chunk_id"] == "doc-1:chunk:0"
    assert result["claims"][0]["document_id"] == "doc-1"
    assert result["claims"][0]["artifact_id"] == "artifact://doc-1"
    assert result["claims"][0]["filename"] == "doc-1.md"
    assert result["claims"][0]["span"] == {"start": 0, "end": 30}
    assert result["claims"][0]["order"] == 0
    assert result["claims"][0]["source_text"] == "Alice founded Acme in Bogota."
    assert result["fragment_artifact_map"][result["fragment_ids"][0]] == "artifact://doc-1"
    assert result["claim_set"]["claims"][0]["cas_id"] == result["fragment_ids"][0]
    assert result["claim_set"]["claims"][0]["mime_type"] == "application/vnd.ikam.claim+json"
    assert result["claim_set"]["claims"][0]["value"]["entity_relationship_fragment_id"] == "frag-ir-1"


def test_claims_operator_registry_registers_operator(monkeypatch) -> None:
    from modelado.operators import ClaimsOperator

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

    assert isinstance(registry.get("modelado/operators/claims"), ClaimsOperator)
