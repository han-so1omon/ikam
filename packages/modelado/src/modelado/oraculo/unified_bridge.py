"""Compatibility bridges from unified AIClient to legacy protocols."""

from __future__ import annotations

import asyncio
from dataclasses import dataclass

from ikam.oraculo.judge import Judgment, JudgeQuery, JudgeProtocol

from modelado.oraculo.factory import create_ai_client_from_env
from modelado.oraculo.ai_client import GenerateRequest, JudgeRequest, AIClient


@dataclass
class CallModelResult:
    output: str


class UnifiedCallModelClient:
    """Adapter exposing `call_model(...)` for forja components."""

    def __init__(self, ai_client: AIClient | None = None, default_model: str | None = None) -> None:
        self._ai_client = ai_client or create_ai_client_from_env()
        self._default_model = default_model

    @classmethod
    def from_env(cls) -> "UnifiedCallModelClient":
        return cls()

    async def call_model(self, prompt: str, model: str, temperature: float) -> CallModelResult:
        response = await self._ai_client.generate(
            GenerateRequest(
                messages=[
                    {"role": "system", "content": "Return valid JSON only when JSON is requested."},
                    {"role": "user", "content": prompt},
                ],
                model=model or self._default_model or "",
                temperature=temperature,
                metadata={"component": "UnifiedCallModelClient.call_model"},
            )
        )
        return CallModelResult(output=response.text)


class UnifiedJudge(JudgeProtocol):
    """JudgeProtocol implementation backed by unified LLM client."""

    def __init__(self, model: str, ai_client: AIClient | None = None) -> None:
        self._ai_client = ai_client or create_ai_client_from_env()
        self._model = model

    def judge(self, query: JudgeQuery) -> Judgment:
        response = asyncio.run(
            self._ai_client.judge(
                JudgeRequest(
                    question=query.question,
                    context=query.context,
                    candidates=query.candidates,
                    model=self._model,
                    metadata={"component": "UnifiedJudge.judge"},
                )
            )
        )
        return Judgment(
            score=response.score,
            reasoning=response.reasoning,
            facts_found=response.facts_found,
            metadata=response.metadata,
        )
