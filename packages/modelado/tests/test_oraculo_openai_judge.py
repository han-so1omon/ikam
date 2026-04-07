"""Tests for OpenAIJudge — modelado adapter implementing JudgeProtocol."""
from __future__ import annotations

import os
import json

import pytest

from ikam.oraculo.judge import JudgeProtocol, JudgeQuery, Judgment


needs_openai = pytest.mark.skipif(
    not os.environ.get("OPENAI_API_KEY"),
    reason="OPENAI_API_KEY not set",
)


def test_openai_judge_satisfies_protocol():
    """OpenAIJudge must be a runtime-checkable JudgeProtocol implementation."""
    from modelado.oraculo.openai_judge import OpenAIJudge

    judge = OpenAIJudge(model="gpt-4o-mini")
    assert isinstance(judge, JudgeProtocol)


def test_openai_judge_returns_judgment_shape_with_mock(monkeypatch, tmp_path):
    """Verify OpenAIJudge parses OpenAI response into Judgment dataclass."""
    from modelado.oraculo.openai_judge import OpenAIJudge

    # Mock the OpenAI client
    class FakeMessage:
        def __init__(self, content):
            self.content = content

    class FakeChoice:
        def __init__(self, content):
            self.message = FakeMessage(content)

    class FakeCompletion:
        def __init__(self, content):
            self.choices = [FakeChoice(content)]

    class FakeChat:
        class completions:
            @staticmethod
            def create(**kwargs):
                return FakeCompletion(json.dumps({
                    "score": 0.85,
                    "reasoning": "The entity Maya Chen is present.",
                    "facts_found": ["Maya Chen founded B&B"],
                    "metadata": {"confidence": "high"},
                }))

    class FakeClient:
        chat = FakeChat()

    judge = OpenAIJudge(model="gpt-4o-mini")
    judge._client = FakeClient()

    trace_path = tmp_path / "judge-trace.jsonl"
    monkeypatch.setenv("IKAM_LLM_TRACE_FILE", str(trace_path))

    query = JudgeQuery(question="Is Maya Chen mentioned?", context={"text": "Maya Chen founded B&B"})
    result = judge.judge(query)

    assert isinstance(result, Judgment)
    assert 0.0 <= result.score <= 1.0
    assert isinstance(result.reasoning, str)
    assert result.reasoning == "The entity Maya Chen is present."
    assert result.facts_found == ["Maya Chen founded B&B"]

    lines = trace_path.read_text(encoding="utf-8").strip().splitlines()
    assert len(lines) >= 2
    request = json.loads(lines[-2])
    response = json.loads(lines[-1])
    assert request["phase"] == "request"
    assert response["phase"] == "response"


@needs_openai
def test_openai_judge_live_call():
    """Integration test: verify real OpenAI call returns valid Judgment."""
    from modelado.oraculo.openai_judge import OpenAIJudge

    judge = OpenAIJudge(model="gpt-4o-mini")
    query = JudgeQuery(
        question="Score how well this text mentions the entity 'Maya Chen'.",
        context={"text": "Maya Chen founded Bramble & Bitters in 2024."},
    )
    result = judge.judge(query)
    assert isinstance(result, Judgment)
    assert 0.0 <= result.score <= 1.0
    assert isinstance(result.reasoning, str)
