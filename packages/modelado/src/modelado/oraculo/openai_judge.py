"""OpenAI-backed judge implementing JudgeProtocol."""
from __future__ import annotations

import json
from typing import Any

from ikam.oraculo.judge import JudgeProtocol, JudgeQuery, Judgment
from modelado.oraculo.llm_trace import emit_llm_trace


class OpenAIJudge:
    """JudgeProtocol implementation using OpenAI chat completions.

    Sends structured prompts to the model and parses JSON responses
    into Judgment dataclasses. Client is created lazily on first use
    to allow construction without OPENAI_API_KEY (for protocol checks
    and monkeypatching in tests).
    """

    def __init__(self, model: str = "gpt-4o-mini"):
        self._client: Any = None
        self._model = model

    def _get_client(self) -> Any:
        if self._client is None:
            from openai import OpenAI
            self._client = OpenAI()
        return self._client

    def judge(self, query: JudgeQuery) -> Judgment:
        """Send query to OpenAI and parse response into Judgment."""
        system_prompt = (
            "You are an evaluation judge. Respond with JSON only.\n"
            "Required fields:\n"
            '  "score": float 0.0-1.0\n'
            '  "reasoning": string explaining your judgment\n'
            '  "facts_found": list of strings (facts found in context), or null\n'
            '  "metadata": dict with any additional structured data\n'
        )
        user_prompt = self._build_user_prompt(query)

        request_payload = {
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt},
            ],
            "temperature": 0.0,
            "response_format": {"type": "json_object"},
        }
        emit_llm_trace(
            provider="openai",
            operation="chat.completions",
            model=self._model,
            phase="request",
            request_payload=request_payload,
            metadata={"component": "OpenAIJudge", "question": query.question},
        )
        try:
            response = self._get_client().chat.completions.create(
                model=self._model,
                messages=request_payload["messages"],
                temperature=0.0,
                response_format={"type": "json_object"},
            )
            raw_text = response.choices[0].message.content or "{}"
            emit_llm_trace(
                provider="openai",
                operation="chat.completions",
                model=self._model,
                phase="response",
                response_payload={"content": raw_text},
                metadata={"component": "OpenAIJudge", "question": query.question},
            )
            return self._parse_judgment(raw_text)
        except Exception as exc:
            emit_llm_trace(
                provider="openai",
                operation="chat.completions",
                model=self._model,
                phase="error",
                metadata={"component": "OpenAIJudge", "question": query.question, "error": str(exc)},
            )
            raise

    def _build_user_prompt(self, query: JudgeQuery) -> str:
        parts = [f"Question: {query.question}"]
        if query.context:
            parts.append(f"Context: {json.dumps(query.context, default=str)}")
        if query.candidates:
            parts.append(f"Candidates: {json.dumps(query.candidates, default=str)}")
        return "\n\n".join(parts)

    def _parse_judgment(self, raw_text: str) -> Judgment:
        try:
            data = json.loads(raw_text)
        except json.JSONDecodeError:
            return Judgment(
                score=0.0,
                reasoning=f"Failed to parse JSON: {raw_text[:200]}",
                facts_found=None,
                metadata={"raw": raw_text[:500]},
            )

        return Judgment(
            score=float(data.get("score", 0.0)),
            reasoning=str(data.get("reasoning", "")),
            facts_found=data.get("facts_found"),
            metadata=data.get("metadata", {}),
        )
