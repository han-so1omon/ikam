"""OpenAI provider adapter for unified AIClient."""

from __future__ import annotations

import json
import os
import asyncio
from typing import Any

from modelado.oraculo.ai_client import (
    EmbedRequest,
    EmbedResponse,
    GenerateRequest,
    GenerateResponse,
    JudgeRequest,
    JudgeResponse,
)
from modelado.oraculo.llm_trace import emit_llm_trace


class OpenAIUnifiedAIClient:
    """Unified generate/embed/judge adapter backed by OpenAI SDK."""

    provider = "openai"

    def __init__(self, *, model: str, embed_model: str, judge_model: str, api_key: str | None = None) -> None:
        self.model = model
        self.embed_model = embed_model
        self.judge_model = judge_model
        self._api_key = (api_key or os.getenv("OPENAI_API_KEY", "")).strip()
        if not self._api_key:
            raise ValueError("OPENAI_API_KEY is required for LLM_PROVIDER=openai")
        self._client: Any = None
        self._client_loop: asyncio.AbstractEventLoop | None = None

    def _get_client(self) -> Any:
        current_loop = asyncio.get_running_loop()
        if self._client is None or self._client_loop is not current_loop:
            from openai import AsyncOpenAI

            self._client = AsyncOpenAI(api_key=self._api_key)
            self._client_loop = current_loop
        return self._client

    async def aclose(self) -> None:
        """Close the underlying client gracefully to avoid connection leaks."""
        if self._client is not None:
            await self._client.close()
            self._client = None
            self._client_loop = None

    async def generate(self, request: GenerateRequest) -> GenerateResponse:
        response_format = request.response_format or {"type": "text"}
        emit_llm_trace(
            provider=self.provider,
            operation="chat.completions",
            model=request.model,
            phase="request",
            request_payload={
                "messages": request.messages,
                "temperature": request.temperature,
                "response_format": response_format,
            },
            metadata={"component": "OpenAIUnifiedAIClient.generate", **request.metadata},
        )
        response = await self._get_client().chat.completions.create(
            model=request.model,
            messages=request.messages,
            temperature=request.temperature,
            response_format=response_format,
            max_tokens=request.max_tokens,
            seed=request.seed,
        )
        text = (response.choices[0].message.content or "").strip()
        usage = {
            "prompt_tokens": int(getattr(response.usage, "prompt_tokens", 0) or 0),
            "completion_tokens": int(getattr(response.usage, "completion_tokens", 0) or 0),
            "total_tokens": int(getattr(response.usage, "total_tokens", 0) or 0),
        }
        emit_llm_trace(
            provider=self.provider,
            operation="chat.completions",
            model=request.model,
            phase="response",
            response_payload={"content": text, "usage": usage},
            metadata={"component": "OpenAIUnifiedAIClient.generate", **request.metadata},
        )
        return GenerateResponse(
            text=text,
            provider=self.provider,
            model=request.model,
            usage=usage,
            raw={"id": getattr(response, "id", None)},
        )

    async def embed(self, request: EmbedRequest) -> EmbedResponse:
        emit_llm_trace(
            provider=self.provider,
            operation="embeddings.create",
            model=request.model,
            phase="request",
            request_payload={"input_count": len(request.texts), "inputs": request.texts[:3]},
            metadata={"component": "OpenAIUnifiedAIClient.embed", **request.metadata},
        )
        response = await self._get_client().embeddings.create(model=request.model, input=request.texts)
        vectors = [item.embedding for item in response.data]
        usage = {
            "prompt_tokens": int(getattr(response.usage, "prompt_tokens", 0) or 0),
            "total_tokens": int(getattr(response.usage, "total_tokens", 0) or 0),
        }
        emit_llm_trace(
            provider=self.provider,
            operation="embeddings.create",
            model=request.model,
            phase="response",
            response_payload={"embedding_count": len(vectors), "usage": usage},
            metadata={"component": "OpenAIUnifiedAIClient.embed", **request.metadata},
        )
        return EmbedResponse(
            vectors=vectors,
            provider=self.provider,
            model=request.model,
            usage=usage,
            raw={"model": getattr(response, "model", request.model)},
        )

    async def judge(self, request: JudgeRequest) -> JudgeResponse:
        prompt = [
            {"role": "system", "content": "You are an evaluation judge. Return JSON with score, reasoning, facts_found, metadata."},
            {
                "role": "user",
                "content": json.dumps(
                    {
                        "question": request.question,
                        "context": request.context,
                        "candidates": request.candidates,
                    },
                    ensure_ascii=True,
                ),
            },
        ]
        generated = await self.generate(
            GenerateRequest(
                messages=prompt,
                model=request.model or self.judge_model,
                temperature=0.0,
                response_format={"type": "json_object"},
                metadata={"component": "OpenAIUnifiedAIClient.judge", **request.metadata},
            )
        )
        try:
            parsed = json.loads(generated.text or "{}")
        except json.JSONDecodeError:
            parsed = {
                "score": 0.0,
                "reasoning": f"Unparseable judge response: {generated.text[:200]}",
                "facts_found": None,
                "metadata": {"raw": generated.text[:500]},
            }

        return JudgeResponse(
            score=float(parsed.get("score", 0.0)),
            reasoning=str(parsed.get("reasoning", "")),
            facts_found=parsed.get("facts_found"),
            provider=self.provider,
            model=request.model or self.judge_model,
            metadata=parsed.get("metadata", {}),
            raw={"generated": generated.raw},
        )
