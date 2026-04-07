from __future__ import annotations

from dataclasses import asdict

from modelado.oraculo import (
    EmbedRequest,
    EmbedResponse,
    GenerateRequest,
    GenerateResponse,
    JudgeRequest,
    JudgeResponse,
)


def test_generate_request_response_shape() -> None:
    request = GenerateRequest(
        messages=[{"role": "user", "content": "hello"}],
        model="gpt-4o-mini",
        temperature=0.2,
        response_format={"type": "json_object"},
    )
    response = GenerateResponse(
        text='{"ok": true}',
        provider="openai",
        model="gpt-4o-mini",
        usage={"prompt_tokens": 5, "completion_tokens": 3},
    )

    assert request.messages[0]["role"] == "user"
    assert request.response_format == {"type": "json_object"}
    assert response.usage["completion_tokens"] == 3
    assert asdict(response)["provider"] == "openai"


def test_embed_request_response_shape() -> None:
    request = EmbedRequest(texts=["a", "b"], model="text-embedding-3-large")
    response = EmbedResponse(
        vectors=[[0.1, 0.2], [0.3, 0.4]],
        provider="openai",
        model="text-embedding-3-large",
    )

    assert request.texts == ["a", "b"]
    assert len(response.vectors) == 2
    assert response.model == "text-embedding-3-large"


def test_judge_request_response_shape() -> None:
    request = JudgeRequest(
        question="Is this coherent?",
        context={"text": "sample"},
        candidates=[{"id": "a"}],
        model="gpt-4o-mini",
    )
    response = JudgeResponse(
        score=0.8,
        reasoning="Clear and well-formed",
        facts_found=["contains title"],
        provider="openai",
        model="gpt-4o-mini",
        metadata={"rubric": "v1"},
    )

    assert request.context == {"text": "sample"}
    assert request.candidates and request.candidates[0]["id"] == "a"
    assert response.score == 0.8
    assert response.metadata["rubric"] == "v1"
