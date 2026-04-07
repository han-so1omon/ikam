import asyncio
import types

import pytest

from modelado.config.llm_config import LLMConfig, LLMModel
from modelado.generation.model_call_client import ModelCallClient, ModelCallCache


@pytest.mark.asyncio
async def test_returns_cached_result(monkeypatch):
    config = LLMConfig(enable_llm_extraction=False)
    cache = ModelCallCache()
    client = ModelCallClient(config, cache=cache)

    calls = {"count": 0}

    async def fake_invoke(prompt, model, seed, max_tokens, temperature):
        calls["count"] += 1
        return "out", 100, 50

    monkeypatch.setattr(client, "_invoke_model", fake_invoke)

    # First call populates cache
    result1 = await client.call_model("hello", seed=42)
    # Second call should hit cache
    result2 = await client.call_model("hello", seed=42)

    assert calls["count"] == 1
    assert result1.output == "out"
    assert result2.cached is True
    assert result1.prompt_hash == result2.prompt_hash


@pytest.mark.asyncio
async def test_seed_changes_output(monkeypatch):
    config = LLMConfig(enable_llm_extraction=False)
    client = ModelCallClient(config)

    async def fake_invoke(prompt, model, seed, max_tokens, temperature):
        return f"{prompt}-{seed}", 10, 5

    monkeypatch.setattr(client, "_invoke_model", fake_invoke)

    res_a = await client.call_model("prompt", seed=1)
    res_b = await client.call_model("prompt", seed=2)

    assert res_a.output != res_b.output
    assert res_a.seed == 1
    assert res_b.seed == 2


@pytest.mark.asyncio
async def test_cost_estimation(monkeypatch):
    config = LLMConfig(enable_llm_extraction=False, model=LLMModel.GPT_4O_MINI)
    client = ModelCallClient(config)

    async def fake_invoke(prompt, model, seed, max_tokens, temperature):
        # 1k tokens in/out for simple cost calc
        return "x", 1000, 1000

    monkeypatch.setattr(client, "_invoke_model", fake_invoke)
    result = await client.call_model("prompt")

    # GPT-4o-mini: input $0.15/MTok, output $0.60/MTok
    expected_cost = (0.15 / 1000) + (0.60 / 1000)
    assert pytest.approx(result.cost_usd, rel=1e-3) == expected_cost


@pytest.mark.asyncio
async def test_missing_client_raises():
    config = LLMConfig(enable_llm_extraction=False, model=LLMModel.GPT_4O_MINI)
    client = ModelCallClient(config)
    with pytest.raises(RuntimeError):
        await client._call_openai("prompt", "gpt-4o-mini", None, 10, 0.3)
