from __future__ import annotations

import asyncio
import json
import os

from ikam_perf_report.benchmarks import semantic_pipeline
from modelado.oraculo.ai_client import GenerateResponse


def _set_required_llm_env(monkeypatch):
    monkeypatch.setenv("LLM_PROVIDER", "openai")
    monkeypatch.setenv("LLM_MODEL", "gpt-4o-mini")
    monkeypatch.setenv("LLM_EMBED_MODEL", "text-embedding-3-large")
    monkeypatch.setenv("LLM_JUDGE_MODEL", "gpt-4o-mini")


def test_semantic_pipeline_requires_real_api_key(monkeypatch):
    _set_required_llm_env(monkeypatch)
    monkeypatch.delenv("OPENAI_API_KEY", raising=False)
    try:
        semantic_pipeline.run_semantic_pipeline("Revenue depends on pricing")
        assert False, "Expected ValueError when OPENAI_API_KEY is missing"
    except ValueError as exc:
        assert "OPENAI_API_KEY" in str(exc)


def test_semantic_pipeline_relation_contract_has_rationale_and_evidence(monkeypatch):
    _set_required_llm_env(monkeypatch)
    monkeypatch.setenv("OPENAI_API_KEY", os.getenv("OPENAI_API_KEY", "test-key") or "test-key")

    async def _fake_extract(intent: str, *, model: str, ai_client=None, api_key=None):
        return {
            "entities": [
                {"label": "Revenue", "kind": "metric", "confidence": 0.9, "evidence": ["Revenue grew 20%"]},
                {"label": "Pricing", "kind": "driver", "confidence": 0.86, "evidence": ["Price increase in Q3"]},
            ],
            "relations": [
                {
                    "source_label": "Revenue",
                    "target_label": "Pricing",
                    "kind": "value_driver",
                    "confidence": 0.84,
                    "rationale": "Revenue movement is explained by pricing changes",
                    "evidence": ["Revenue grew 20%", "Price increase in Q3"],
                }
            ],
        }

    monkeypatch.setattr(semantic_pipeline, "_extract_semantic_graph_with_llm", _fake_extract)
    result = semantic_pipeline.run_semantic_pipeline("Revenue grew after pricing changes")

    assert len(result["entities"]) >= 2
    assert len(result["relations"]) >= 1
    relation = result["relations"][0]
    assert relation["kind"] != "semantic_link"
    assert relation["rationale"]
    assert relation["evidence"]


def test_semantic_pipeline_emits_llm_trace(monkeypatch, tmp_path):
    _set_required_llm_env(monkeypatch)
    monkeypatch.setenv("OPENAI_API_KEY", os.getenv("OPENAI_API_KEY", "test-key") or "test-key")
    monkeypatch.setenv("IKAM_LLM_TRACE_FILE", str(tmp_path / "semantic-trace.jsonl"))

    class FakeUnifiedClient:
        async def generate(self, request):
            return GenerateResponse(
                text=json.dumps({"entities": [], "relations": []}),
                provider="openai",
                model=request.model,
            )

    monkeypatch.setattr(
        semantic_pipeline,
        "create_ai_client_from_env",
        lambda: FakeUnifiedClient(),
    )

    payload = asyncio.run(
        semantic_pipeline._extract_semantic_graph_with_llm(
            "Revenue depends on pricing", model="gpt-4o-mini"
        )
    )
    assert payload == {"entities": [], "relations": []}

    trace_path = tmp_path / "semantic-trace.jsonl"
    lines = trace_path.read_text(encoding="utf-8").strip().splitlines()
    assert len(lines) >= 2
    assert json.loads(lines[-2])["phase"] == "request"
    assert json.loads(lines[-1])["phase"] == "response"
