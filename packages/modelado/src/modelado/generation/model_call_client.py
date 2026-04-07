"""Deterministic model call client with caching.

This client wraps LLM calls with:
- Prompt hashing + optional seeding for determinism
- In-memory cache keyed by (model, prompt_hash, seed)
- Cost/latency/token tracking

It intentionally avoids provider-specific SDK dependencies during tests; the
actual network call functions can be monkeypatched.
"""
from __future__ import annotations

import asyncio
import hashlib
import logging
import time
from dataclasses import dataclass, replace
from datetime import datetime, timedelta
from typing import Any, Callable, Dict, Optional, Tuple

from modelado.config.llm_config import LLMConfig, LLMModel
from ikam.graph import _cas_hex

log = logging.getLogger(__name__)


@dataclass
class ModelCallResult:
    output: str
    model: str
    prompt_hash: str
    seed: Optional[int]
    cost_usd: float
    tokens_input: int
    tokens_output: int
    latency_ms: float
    cached: bool
    timestamp: datetime
    batch_id: Optional[int] = None
    queue_position: Optional[int] = None
    item_index: Optional[int] = None

    def to_dict(self) -> Dict[str, Any]:
        return {
            "output": self.output,
            "model": self.model,
            "prompt_hash": self.prompt_hash,
            "seed": self.seed,
            "cost_usd": self.cost_usd,
            "tokens_input": self.tokens_input,
            "tokens_output": self.tokens_output,
            "latency_ms": self.latency_ms,
            "cached": self.cached,
            "timestamp": self.timestamp.isoformat(),
            "batch_id": self.batch_id,
            "queue_position": self.queue_position,
            "item_index": self.item_index,
        }

    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> "ModelCallResult":
        return cls(
            output=data["output"],
            model=data["model"],
            prompt_hash=data["prompt_hash"],
            seed=data.get("seed"),
            cost_usd=data.get("cost_usd", 0.0),
            tokens_input=int(data.get("tokens_input", 0)),
            tokens_output=int(data.get("tokens_output", 0)),
            latency_ms=float(data.get("latency_ms", 0.0)),
            cached=bool(data.get("cached", False)),
            timestamp=datetime.fromisoformat(data["timestamp"])
            if isinstance(data.get("timestamp"), str)
            else datetime.utcnow(),
            batch_id=data.get("batch_id"),
            queue_position=data.get("queue_position"),
            item_index=data.get("item_index"),
        )


class ModelCallCache:
    """Simple in-memory cache keyed by (model, prompt_hash, seed)."""

    def __init__(self, ttl_seconds: Optional[int] = None):
        self._store: Dict[str, Tuple[ModelCallResult, float]] = {}
        self._ttl = ttl_seconds
        self._lock = asyncio.Lock()

    def _key(self, model: str, prompt_hash: str, seed: Optional[int]) -> str:
        seed_part = "none" if seed is None else str(seed)
        return _cas_hex(f"{model}:{prompt_hash}:{seed_part}".encode("utf-8"))

    async def get(self, model: str, prompt_hash: str, seed: Optional[int]) -> Optional[ModelCallResult]:
        key = self._key(model, prompt_hash, seed)
        async with self._lock:
            item = self._store.get(key)
            if not item:
                return None
            result, inserted_at = item
            if self._ttl is not None and (time.time() - inserted_at) > self._ttl:
                self._store.pop(key, None)
                return None
            return replace(result, cached=True)

    async def set(self, result: ModelCallResult) -> None:
        key = self._key(result.model, result.prompt_hash, result.seed)
        async with self._lock:
            self._store[key] = (result, time.time())


class ModelCallClient:
    """Deterministic model call client with caching and cost tracking."""

    def __init__(
        self,
        config: LLMConfig,
        *,
        cache: Optional[ModelCallCache] = None,
        openai_client: Any = None,
        anthropic_client: Any = None,
        clock: Callable[[], float] = time.monotonic,
    ):
        self.config = config
        self.cache = cache or ModelCallCache()
        self.openai_client = openai_client
        self.anthropic_client = anthropic_client
        self.clock = clock

    async def call_model(
        self,
        prompt: str,
        *,
        model: Optional[LLMModel] = None,
        seed: Optional[int] = None,
        max_tokens: Optional[int] = None,
        temperature: Optional[float] = None,
    ) -> ModelCallResult:
        chosen_model = model or self.config.model
        prompt_hash = self._hash_prompt(prompt)

        cached = await self.cache.get(chosen_model.value, prompt_hash, seed)
        if cached:
            return cached

        started = self.clock()
        output, tokens_in, tokens_out = await self._invoke_model(
            prompt,
            chosen_model,
            seed=seed,
            max_tokens=max_tokens or self.config.max_tokens,
            temperature=temperature if temperature is not None else self.config.temperature,
        )
        latency_ms = (self.clock() - started) * 1000.0

        cost_usd = self._estimate_cost(chosen_model, tokens_in, tokens_out)
        result = ModelCallResult(
            output=output,
            model=chosen_model.value,
            prompt_hash=prompt_hash,
            seed=seed,
            cost_usd=cost_usd,
            tokens_input=tokens_in,
            tokens_output=tokens_out,
            latency_ms=latency_ms,
            cached=False,
            timestamp=datetime.utcnow(),
        )
        await self.cache.set(result)
        return result

    def _hash_prompt(self, prompt: str) -> str:
        return hashlib.sha256(prompt.encode("utf-8")).hexdigest()

    async def _invoke_model(
        self,
        prompt: str,
        model: LLMModel,
        *,
        seed: Optional[int],
        max_tokens: int,
        temperature: float,
    ) -> Tuple[str, int, int]:
        """Invoke provider SDK. Separated for test monkeypatching."""
        if model in {LLMModel.GPT_4O_MINI, LLMModel.GPT_4}:
            return await self._call_openai(prompt, model.value, seed, max_tokens, temperature)
        if model in {LLMModel.CLAUDE_35_HAIKU, LLMModel.CLAUDE_3_OPUS}:
            return await self._call_anthropic(prompt, model.value, seed, max_tokens, temperature)
        raise ValueError(f"Unsupported model: {model}")

    async def _call_openai(
        self,
        prompt: str,
        model_name: str,
        seed: Optional[int],
        max_tokens: int,
        temperature: float,
    ) -> Tuple[str, int, int]:
        if self.openai_client is None:
            raise RuntimeError("OpenAI client not configured")
        messages = [
            {"role": "user", "content": prompt},
        ]
        response = await self.openai_client.chat.completions.create(
            model=model_name,
            messages=messages,
            max_tokens=max_tokens,
            temperature=temperature,
            seed=seed,
        )
        output = response.choices[0].message.content
        usage = getattr(response, "usage", None) or {}
        tokens_in = int(getattr(usage, "prompt_tokens", usage.get("prompt_tokens", 0)))
        tokens_out = int(getattr(usage, "completion_tokens", usage.get("completion_tokens", 0)))
        return output, tokens_in, tokens_out

    async def _call_anthropic(
        self,
        prompt: str,
        model_name: str,
        seed: Optional[int],
        max_tokens: int,
        temperature: float,
    ) -> Tuple[str, int, int]:
        if self.anthropic_client is None:
            raise RuntimeError("Anthropic client not configured")
        response = await self.anthropic_client.messages.create(
            model=model_name,
            max_tokens=max_tokens,
            temperature=temperature,
            messages=[{"role": "user", "content": prompt}],
            metadata={"seed": seed} if seed is not None else None,
        )
        output = response.content[0].text if hasattr(response, "content") else str(response)
        usage = getattr(response, "usage", None) or {}
        tokens_in = int(getattr(usage, "input_tokens", usage.get("input_tokens", 0)))
        tokens_out = int(getattr(usage, "output_tokens", usage.get("output_tokens", 0)))
        return output, tokens_in, tokens_out

    def _estimate_cost(self, model: LLMModel, tokens_in: int, tokens_out: int) -> float:
        # Costs per 1M tokens (matches LLMConfig pricing hints)
        costs_per_million = {
            LLMModel.GPT_4O_MINI: {"input": 0.15, "output": 0.60},
            LLMModel.GPT_4: {"input": 30.0, "output": 60.0},
            LLMModel.CLAUDE_35_HAIKU: {"input": 0.80, "output": 4.00},
            LLMModel.CLAUDE_3_OPUS: {"input": 15.0, "output": 75.0},
        }
        costs = costs_per_million.get(model)
        if not costs:
            return 0.0
        cost_in = (tokens_in / 1_000_000) * costs["input"]
        cost_out = (tokens_out / 1_000_000) * costs["output"]
        return round(cost_in + cost_out, 6)
