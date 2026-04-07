from __future__ import annotations

import asyncio
import sys
from types import SimpleNamespace

import pytest

from modelado.oraculo.ai_client import EmbedRequest, GenerateRequest, JudgeRequest
from modelado.oraculo.providers.openai_client import OpenAIUnifiedAIClient


class _FakeChatCompletions:
    async def create(self, **kwargs):
        content = kwargs["messages"][1]["content"]
        if kwargs.get("response_format", {}).get("type") == "json_object":
            payload = '{"score": 0.9, "reasoning": "good", "facts_found": ["a"], "metadata": {"k": "v"}}'
        else:
            payload = f"echo:{content}"
        return SimpleNamespace(
            id="chat-1",
            choices=[SimpleNamespace(message=SimpleNamespace(content=payload))],
            usage=SimpleNamespace(prompt_tokens=11, completion_tokens=7, total_tokens=18),
        )


class _FakeEmbeddings:
    async def create(self, **kwargs):
        input_texts = kwargs["input"]
        data = [SimpleNamespace(embedding=[float(index), float(index + 1)]) for index, _ in enumerate(input_texts)]
        return SimpleNamespace(
            data=data,
            model=kwargs["model"],
            usage=SimpleNamespace(prompt_tokens=6, total_tokens=6),
        )


@pytest.mark.asyncio
async def test_openai_adapter_generate_embed_judge(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("OPENAI_API_KEY", "sk-test")
    client = OpenAIUnifiedAIClient(
        model="gpt-4o-mini",
        embed_model="text-embedding-3-large",
        judge_model="gpt-4o-mini",
    )
    client._client = SimpleNamespace(
        chat=SimpleNamespace(completions=_FakeChatCompletions()),
        embeddings=_FakeEmbeddings(),
    )

    generated = await client.generate(
        GenerateRequest(
            messages=[{"role": "system", "content": "S"}, {"role": "user", "content": "hello"}],
            model="gpt-4o-mini",
            temperature=0.1,
        )
    )
    assert generated.provider == "openai"
    assert generated.model == "gpt-4o-mini"
    assert generated.text.startswith("echo:")

    embedded = await client.embed(
        EmbedRequest(texts=["alpha", "beta"], model="text-embedding-3-large")
    )
    assert embedded.provider == "openai"
    assert len(embedded.vectors) == 2

    judged = await client.judge(
        JudgeRequest(question="Is this correct?", context={"a": 1}, candidates=[{"id": "x"}], model="gpt-4o-mini")
    )
    assert judged.provider == "openai"
    assert judged.score == 0.9
    assert judged.reasoning == "good"


def test_openai_adapter_requires_api_key(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("OPENAI_API_KEY", raising=False)
    with pytest.raises(ValueError) as exc_info:
        OpenAIUnifiedAIClient(
            model="gpt-4o-mini",
            embed_model="text-embedding-3-large",
            judge_model="gpt-4o-mini",
        )
    assert "OPENAI_API_KEY" in str(exc_info.value)


def test_openai_adapter_recreates_async_client_when_event_loop_changes(monkeypatch: pytest.MonkeyPatch) -> None:
    class _LoopBoundChatCompletions:
        def __init__(self, owner: "_LoopBoundAsyncOpenAI") -> None:
            self._owner = owner

        async def create(self, **kwargs):
            current_loop = asyncio.get_running_loop()
            if self._owner.bound_loop is not current_loop:
                raise RuntimeError("Event loop is closed")
            content = kwargs["messages"][1]["content"]
            return SimpleNamespace(
                id="chat-loop-bound",
                choices=[SimpleNamespace(message=SimpleNamespace(content=f"echo:{content}"))],
                usage=SimpleNamespace(prompt_tokens=1, completion_tokens=1, total_tokens=2),
            )

    class _LoopBoundAsyncOpenAI:
        created_loops: list[asyncio.AbstractEventLoop] = []

        def __init__(self, **_kwargs) -> None:
            self.bound_loop = asyncio.get_running_loop()
            self.chat = SimpleNamespace(completions=_LoopBoundChatCompletions(self))
            self.embeddings = SimpleNamespace(create=None)
            type(self).created_loops.append(self.bound_loop)

        async def close(self) -> None:
            return None

    fake_openai = SimpleNamespace(AsyncOpenAI=_LoopBoundAsyncOpenAI)
    monkeypatch.setenv("OPENAI_API_KEY", "sk-test")
    monkeypatch.setitem(sys.modules, "openai", fake_openai)

    client = OpenAIUnifiedAIClient(
        model="gpt-4o-mini",
        embed_model="text-embedding-3-large",
        judge_model="gpt-4o-mini",
    )

    first = asyncio.run(
        client.generate(
            GenerateRequest(
                messages=[{"role": "system", "content": "S"}, {"role": "user", "content": "first"}],
                model="gpt-4o-mini",
                temperature=0.0,
            )
        )
    )
    second = asyncio.run(
        client.generate(
            GenerateRequest(
                messages=[{"role": "system", "content": "S"}, {"role": "user", "content": "second"}],
                model="gpt-4o-mini",
                temperature=0.0,
            )
        )
    )

    assert first.text == "echo:first"
    assert second.text == "echo:second"
    assert len(_LoopBoundAsyncOpenAI.created_loops) == 2
