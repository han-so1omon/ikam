from __future__ import annotations

import sys
from pathlib import Path

import pytest


ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "src"
if str(SRC) not in sys.path:
    sys.path.insert(0, str(SRC))


def _sample_payload() -> dict[str, object]:
    return {
        "artifact_bundle": {
            "corpus_id": "project/corpus",
            "artifacts": [
                {"artifact_id": "project/a", "file_name": "a.md", "mime_type": "text/markdown"},
                {"artifact_id": "project/b", "file_name": "b.md", "mime_type": "text/markdown"},
            ],
        },
        "map_definition": {
            "goal": "Build a map for cross-artifact semantic comparison",
            "allowed_profiles": ["modelado/prose-backbone@1", "modelado/reasoning@1"],
            "max_nodes": 20,
            "max_depth": 3,
        },
        "context": {"project_id": "proj", "case_id": "case-1"},
    }


def test_generate_structural_map_returns_subgraph_and_segments(monkeypatch: pytest.MonkeyPatch) -> None:
    from modelado.oraculo.ai_client import GenerateResponse
    from mcp_ikam.tools import map_generation

    class FakeClient:
        async def generate(self, request):  # noqa: ANN001
            return GenerateResponse(
                text=(
                    '{"map_subgraph":{'
                    '"root_node_id":"map:root",'
                    '"nodes":['
                    '{"id":"map:root","title":"Corpus","kind":"corpus"},'
                    '{"id":"map:seg:a","title":"Segment A","kind":"segment"}'
                    '],'
                    '"relationships":[{"type":"map_contains","source":"map:root","target":"map:seg:a"}]'
                    '},'
                    '"segment_anchors":{'
                    '"map:seg:a":[{"artifact_id":"project/a","locator_type":"section","locator":"#intro","confidence":0.9}]'
                    '},'
                    '"segment_candidates":['
                    '{"segment_id":"map:seg:a","title":"Segment A","artifact_ids":["project/a"],"rationale":"coherent topic"}'
                    '],'
                    '"profile_candidates":{'
                    '"map:seg:a":["modelado/prose-backbone@1"]'
                    '}'
                    '}'
                ),
                provider="openai",
                model="gpt-4o-mini",
            )

    monkeypatch.setattr(map_generation, "create_ai_client_from_env", lambda: FakeClient())

    result = map_generation.generate_structural_map(_sample_payload())

    assert result["map_subgraph"]["root_node_id"] == "map:root"
    assert result["segment_candidates"][0]["segment_id"] == "map:seg:a"
    assert result["segment_anchors"]["map:seg:a"][0]["artifact_id"] == "project/a"
    assert result["map_dna"]["fingerprint"]
    assert [event["phase"] for event in result["trace_events"]] == [
        "request_validated",
        "llm_plan_started",
        "llm_plan_returned",
        "map_subgraph_normalized",
        "segment_candidates_normalized",
        "validation_completed",
    ]
    assert result["trace_events"][1]["model"] == "gpt-4o-mini"
    assert result["trace_events"][2]["provider"] == "openai"


def test_generate_structural_map_derives_defaults_when_optional_fields_missing(monkeypatch: pytest.MonkeyPatch) -> None:
    from modelado.oraculo.ai_client import GenerateResponse
    from mcp_ikam.tools import map_generation

    class FakeClient:
        async def generate(self, request):  # noqa: ANN001
            return GenerateResponse(
                text=(
                    '{"map_subgraph":{'
                    '"root_node_id":"map:root",'
                    '"nodes":['
                    '{"id":"map:root","title":"Corpus","kind":"corpus"},'
                    '{"id":"map:seg:a","title":"Segment A","kind":"segment"}'
                    '],'
                    '"relationships":[{"type":"map_contains","source":"map:root","target":"map:seg:a"}]'
                    '}}'
                ),
                provider="openai",
                model="gpt-4o-mini",
            )

    monkeypatch.setattr(map_generation, "create_ai_client_from_env", lambda: FakeClient())

    result = map_generation.generate_structural_map(_sample_payload())

    assert result["segment_candidates"]
    assert result["profile_candidates"]["map:seg:a"] == [
        "modelado/prose-backbone@1",
        "modelado/reasoning@1",
    ]


def test_generate_structural_map_fails_when_llm_unavailable(monkeypatch: pytest.MonkeyPatch) -> None:
    from mcp_ikam.tools import map_generation

    def _raise() -> None:
        raise ValueError("missing llm configuration")

    monkeypatch.setattr(map_generation, "create_ai_client_from_env", _raise)

    with pytest.raises(RuntimeError, match="map generation failed"):
        map_generation.generate_structural_map(_sample_payload())


def test_generate_structural_map_fails_on_malformed_json(monkeypatch: pytest.MonkeyPatch) -> None:
    from modelado.oraculo.ai_client import GenerateResponse
    from mcp_ikam.tools import map_generation

    class FakeClient:
        async def generate(self, request):  # noqa: ANN001
            return GenerateResponse(text="not-json", provider="openai", model="gpt-4o-mini")

    monkeypatch.setattr(map_generation, "create_ai_client_from_env", lambda: FakeClient())

    with pytest.raises(RuntimeError, match="invalid json"):
        map_generation.generate_structural_map(_sample_payload())
