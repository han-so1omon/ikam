import asyncio
import json

from ikam.forja.normalizer import SemanticNormalizer
from ikam.fragments import CONCEPT_MIME, Fragment


class _Result:
    def __init__(self, output: str):
        self.output = output


class _VariableLLM:
    def __init__(self) -> None:
        self.calls = 0

    async def call_model(self, prompt: str, model: str, temperature: float) -> _Result:
        self.calls += 1
        if self.calls % 2 == 1:
            return _Result(json.dumps(["Revenue", "Gross Margin", "Revenue"]))
        return _Result(json.dumps(["gross margin", "pricing", "Revenue"]))


def _concepts(fragments):
    return [f.value["concept"] for f in fragments]


def test_exploration_lane_permits_variability() -> None:
    llm = _VariableLLM()
    normalizer = SemanticNormalizer(ai_client=llm)
    source = Fragment(value={"text": "Revenue grew with pricing and margin improvements."}, mime_type="text/plain")

    first = asyncio.run(normalizer.normalize(source, mode="explore-graph"))
    second = asyncio.run(normalizer.normalize(source, mode="explore-graph"))

    assert _concepts(first) != _concepts(second)
    assert all(f.mime_type == CONCEPT_MIME for f in first)


def test_commit_lane_canonicalizes_identity_fields_deterministically() -> None:
    llm = _VariableLLM()
    normalizer = SemanticNormalizer(ai_client=llm)
    source = Fragment(value={"text": "Revenue grew with pricing and margin improvements."}, mime_type="text/plain")

    first = asyncio.run(normalizer.normalize(source, mode="commit-strict"))
    second = asyncio.run(normalizer.normalize(source, mode="commit-strict"))

    assert _concepts(first) == _concepts(second)
    assert _concepts(first) == sorted(set(_concepts(first)))
    assert all((f.value or {}).get("mode") == "commit-strict" for f in first)


def test_chunk_boundary_determinism_not_required() -> None:
    llm = _VariableLLM()
    normalizer = SemanticNormalizer(ai_client=llm)
    source_a = Fragment(value={"text": "Revenue grew. Pricing improved.", "chunk_id": "a"}, mime_type="text/plain")
    source_b = Fragment(value={"text": "Revenue grew. Pricing improved.", "chunk_id": "b"}, mime_type="text/plain")

    concepts_a = _concepts(asyncio.run(normalizer.normalize(source_a, mode="explore-fast")))
    concepts_b = _concepts(asyncio.run(normalizer.normalize(source_b, mode="explore-fast")))

    assert concepts_a
    assert concepts_b
