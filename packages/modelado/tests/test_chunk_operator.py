from __future__ import annotations

import json

from modelado.environment_scope import EnvironmentScope
from modelado.operators.core import OperatorEnv, OperatorParams
from modelado.operators.registry import create_default_operator_registry


class _FakeResponse:
    def __init__(self, text: str) -> None:
        self.text = text


class _FakeLLM:
    def __init__(self, text: str) -> None:
        self._text = text

    async def generate(self, _request):
        return _FakeResponse(self._text)


_DEV_SCOPE = EnvironmentScope(ref="refs/heads/run/test")


def test_chunk_operator_returns_grounded_chunks_per_document() -> None:
    from modelado.operators.chunking import ChunkOperator

    operator = ChunkOperator()
    env = OperatorEnv(seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)
    params = OperatorParams(
        name="parse.chunk",
        parameters={
            "artifact_id": "artifact://bundle",
            "source_subgraph_ref": "subgraph://run-1-document-set-step-load",
            "subgraph_ref": "subgraph://run-1-chunk-extraction-set-step-parse",
            "documents": [
                {
                    "id": "doc-1",
                    "source_document_fragment_id": "frag-doc-1",
                    "text": "Alpha intro\n\nBeta details\n\nGamma close",
                    "artifact_id": "artifact://doc-1",
                    "filename": "doc-1.md",
                    "mime_type": "text/markdown",
                },
                {
                    "id": "doc-2",
                    "source_document_fragment_id": "frag-doc-2",
                    "text": "Solo paragraph",
                    "artifact_id": "artifact://doc-2",
                    "filename": "doc-2.md",
                    "mime_type": "text/markdown",
                },
            ],
        },
    )

    result = operator.apply(None, params, env)

    assert result["status"] == "ok"
    assert result["summary"]["document_count"] == 2
    assert result["summary"]["chunk_count"] == 4
    assert result["summary"]["artifact_count"] == 2
    assert [chunk["document_id"] for chunk in result["chunks"]] == [
        "doc-1",
        "doc-1",
        "doc-1",
        "doc-2",
    ]
    assert [chunk["artifact_id"] for chunk in result["chunks"]] == [
        "artifact://doc-1",
        "artifact://doc-1",
        "artifact://doc-1",
        "artifact://doc-2",
    ]
    assert [chunk["source_document_fragment_id"] for chunk in result["chunks"]] == [
        "frag-doc-1",
        "frag-doc-1",
        "frag-doc-1",
        "frag-doc-2",
    ]
    assert [chunk["order"] for chunk in result["chunks"]] == [0, 1, 2, 0]
    assert result["chunks"][0]["text"] == "Alpha intro"
    assert result["chunks"][1]["text"] == "Beta details"
    assert result["chunks"][2]["text"] == "Gamma close"
    assert result["chunks"][3]["text"] == "Solo paragraph"
    assert all(chunk["span"]["start"] < chunk["span"]["end"] for chunk in result["chunks"])
    assert result["document_stats"] == [
        {
            "document_id": "doc-1",
            "source_document_fragment_id": "frag-doc-1",
            "artifact_id": "artifact://doc-1",
            "filename": "doc-1.md",
            "chunk_count": 3,
            "char_count": len("Alpha intro\n\nBeta details\n\nGamma close"),
        },
        {
            "document_id": "doc-2",
            "source_document_fragment_id": "frag-doc-2",
            "artifact_id": "artifact://doc-2",
            "filename": "doc-2.md",
            "chunk_count": 1,
            "char_count": len("Solo paragraph"),
        },
    ]
    assert result["chunk_extraction_set"]["kind"] == "chunk_extraction_set"
    assert result["chunk_extraction_set"]["source_subgraph_ref"] == "subgraph://run-1-document-set-step-load"
    assert result["chunk_extraction_set"]["subgraph_ref"] == "subgraph://run-1-chunk-extraction-set-step-parse"
    assert result["chunk_extraction_set"]["extraction_refs"] == result["fragment_ids"]
    assert result["chunk_extraction_set"]["extractions"][0]["cas_id"] == result["fragment_ids"][0]
    assert result["chunk_extraction_set"]["extractions"][0]["value"]["source_document_fragment_id"] == "frag-doc-1"
    assert result["chunk_extraction_set"]["edges"][0] == {
        "from": f"fragment:{result['fragment_ids'][0]}",
        "to": "fragment:frag-doc-1",
        "edge_label": "knowledge:derives",
    }


def test_chunk_operator_registry_registers_chunk_operator(monkeypatch) -> None:
    from modelado.operators.chunking import ChunkOperator

    created: list[_FakeRegistry] = []

    class _FakeRegistry:
        def __init__(self, _cx: object, _manager: object, namespace: str) -> None:
            self.namespace = namespace
            self.entries: dict[str, object] = {}
            created.append(self)

        def list_keys(self) -> list[str]:
            return sorted(self.entries)

        def register(self, key: str, entry: object) -> None:
            self.entries[key] = entry

        def get(self, key: str) -> object | None:
            return self.entries.get(key)

    monkeypatch.setattr("modelado.operators.registry.OperatorRegistryAdapter", _FakeRegistry)

    registry = create_default_operator_registry(object(), object(), namespace="operators.default.test")

    assert isinstance(registry.get("modelado/operators/chunking"), ChunkOperator)


def test_chunk_operator_uses_llm_grounded_chunk_boundaries_when_available() -> None:
    from modelado.operators.chunking import ChunkOperator

    operator = ChunkOperator()
    env = OperatorEnv(
        seed=42,
        renderer_version="1",
        policy="strict",
        env_scope=_DEV_SCOPE,
        llm=_FakeLLM(json.dumps({"chunks": ["Alpha intro\n\nBeta details", "Gamma close"]})),
    )
    params = OperatorParams(
        name="parse.chunk",
        parameters={
            "artifact_id": "artifact://bundle",
            "source_subgraph_ref": "subgraph://run-1-document-set-step-load",
            "subgraph_ref": "subgraph://run-1-chunk-extraction-set-step-parse",
            "documents": [
                {
                    "id": "doc-1",
                    "source_document_fragment_id": "frag-doc-1",
                    "text": "Alpha intro\n\nBeta details\n\nGamma close",
                    "artifact_id": "artifact://doc-1",
                    "filename": "doc-1.md",
                    "mime_type": "text/markdown",
                },
            ],
        },
    )

    result = operator.apply(None, params, env)

    assert [chunk["text"] for chunk in result["chunks"]] == [
        "Alpha intro\n\nBeta details",
        "Gamma close",
    ]
    assert result["chunks"][0]["span"]["start"] == 0
    assert result["chunks"][0]["span"]["end"] > result["chunks"][0]["span"]["start"]


def test_chunk_operator_emits_document_scoped_chunk_groupings() -> None:
    from modelado.operators.chunking import ChunkOperator

    operator = ChunkOperator()
    env = OperatorEnv(seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)
    params = OperatorParams(
        name="parse.chunk",
        parameters={
            "artifact_id": "artifact://bundle",
            "source_subgraph_ref": "subgraph://run-1-document-set-step-load",
            "subgraph_ref": "subgraph://run-1-chunk-extraction-set-step-parse",
            "documents": [
                {
                    "id": "doc-1",
                    "source_document_fragment_id": "frag-doc-1",
                    "text": "Alpha intro\n\nBeta details",
                    "artifact_id": "artifact://doc-1",
                    "filename": "doc-1.md",
                    "mime_type": "text/markdown",
                },
            ],
        },
    )

    result = operator.apply(None, params, env)

    assert result["chunk_extraction_set"]["document_chunk_sets"] == [
        {
            "kind": "document_chunk_set",
            "document_id": "doc-1",
            "source_document_fragment_id": "frag-doc-1",
            "artifact_id": "artifact://doc-1",
            "filename": "doc-1.md",
            "chunk_refs": result["fragment_ids"],
            "fragment_id": result["document_chunk_sets"][0]["cas_id"],
        }
    ]
