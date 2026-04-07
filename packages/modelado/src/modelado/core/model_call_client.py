"""
Model Call Client: Deterministic model invocation with caching and provenance.

This module provides a unified interface for calling language models with:
- Allowlisted models (GPT-4o-mini, Claude Haiku, etc.)
- Deterministic seeding for reproducibility
- Cost and latency tracking
- CAS-backed output caching (content-addressable)
- Complete provenance recording (seed, model params, cost, latency)

Foundation for model-calling generation strategy.
"""

from __future__ import annotations

import hashlib
import logging
import time
from dataclasses import dataclass, field
from datetime import datetime
from enum import Enum
from typing import Any, Optional
from uuid import uuid4

from pydantic import BaseModel, Field, ConfigDict, PrivateAttr

logger = logging.getLogger(__name__)


class ModelProvider(str, Enum):
    """Allowlisted model providers."""
    OPENAI = "openai"
    ANTHROPIC = "anthropic"
    COHERE = "cohere"


class ModelName(str, Enum):
    """Allowlisted model names with cost/latency profiles."""
    # OpenAI
    GPT_4O_MINI = "gpt-4o-mini"  # ~$0.15/$0.60 per 1M tokens
    GPT_4_TURBO = "gpt-4-turbo"  # ~$10/$30 per 1M tokens (expensive, reserve for complex tasks)
    
    # Anthropic
    CLAUDE_HAIKU = "claude-3-5-haiku"  # ~$0.80/$4.00 per 1M tokens (similar to GPT-4o-mini)
    CLAUDE_OPUS = "claude-3-opus"  # ~$15/$75 per 1M tokens (high capability, use sparingly)


@dataclass
class ModelCallParams:
    """Parameters for a model call."""
    model: ModelName
    prompt: str
    temperature: float = 0.7
    max_tokens: int = 1024
    top_p: float = 1.0
    seed: Optional[int] = None  # Deterministic seeding
    
    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary."""
        return {
            "model": self.model.value,
            "prompt": self.prompt,
            "temperature": self.temperature,
            "max_tokens": self.max_tokens,
            "top_p": self.top_p,
            "seed": self.seed,
        }
    
    def param_hash(self) -> str:
        """Compute hash of model params (excludes seed for cache grouping)."""
        # Hash includes model + prompt + hyperparams but NOT seed
        # (same prompt with different seeds should group in same batch)
        param_str = f"{self.model.value}|{self.prompt}|{self.temperature}|{self.max_tokens}|{self.top_p}"
        return hashlib.sha256(param_str.encode()).hexdigest()


@dataclass
class ModelCallResult:
    """Result of a model call."""
    output: str
    cost_usd: float
    latency_ms: float
    model: ModelName
    seed: Optional[int]
    prompt_hash: str
    output_hash: str
    cached: bool = False
    timestamp: datetime = field(default_factory=datetime.utcnow)


class ModelCallCache:
    """Simple in-memory cache for model outputs."""
    
    def __init__(self, max_entries: int = 1000):
        """Initialize cache."""
        self._cache: dict[str, str] = {}
        self._max_entries = max_entries
        self._hits = 0
        self._misses = 0
    
    def get(self, cache_key: str) -> Optional[str]:
        """Retrieve cached output."""
        if cache_key in self._cache:
            self._hits += 1
            logger.debug(f"Cache hit: {cache_key[:8]}...")
            return self._cache[cache_key]
        self._misses += 1
        return None
    
    def put(self, cache_key: str, output: str) -> None:
        """Store output in cache."""
        if len(self._cache) >= self._max_entries:
            # Simple eviction: remove first entry (FIFO)
            first_key = next(iter(self._cache))
            del self._cache[first_key]
            logger.debug(f"Cache evicted: {first_key[:8]}...")
        
        self._cache[cache_key] = output
        logger.debug(f"Cache put: {cache_key[:8]}...")
    
    def stats(self) -> dict[str, Any]:
        """Get cache statistics."""
        total = self._hits + self._misses
        hit_rate = self._hits / total if total > 0 else 0.0
        return {
            "hits": self._hits,
            "misses": self._misses,
            "hit_rate": hit_rate,
            "entries": len(self._cache),
            "capacity": self._max_entries,
        }
    
    def clear(self) -> None:
        """Clear cache."""
        self._cache.clear()
        self._hits = 0
        self._misses = 0


class ModelCallClient(BaseModel):
    """
    Client for deterministic model invocation with caching and provenance.

    Responsibilities:
    - Validate model/params against allowlist
    - Handle deterministic seeding for reproducibility
    - Track cost and latency
    - Cache outputs in CAS-like structure
    - Record complete provenance (seed, model params, cost, latency)
    """

    model_config = ConfigDict(arbitrary_types_allowed=True)

    client_id: str = Field(default_factory=lambda: str(uuid4()))
    cache: ModelCallCache = Field(default_factory=ModelCallCache)
    
    # Cost profiles: model name → (input_cost_per_1M_tokens, output_cost_per_1M_tokens)
    cost_profiles: dict[ModelName, tuple[float, float]] = Field(
        default_factory=lambda: {
            ModelName.GPT_4O_MINI: (0.150, 0.600),
            ModelName.CLAUDE_HAIKU: (0.800, 4.000),
            ModelName.GPT_4_TURBO: (10.0, 30.0),
            ModelName.CLAUDE_OPUS: (15.0, 75.0),
        }
    )
    
    # Latency profiles: model name → typical latency in ms
    latency_profiles: dict[ModelName, int] = Field(
        default_factory=lambda: {
            ModelName.GPT_4O_MINI: 800,
            ModelName.CLAUDE_HAIKU: 1000,
            ModelName.GPT_4_TURBO: 2000,
            ModelName.CLAUDE_OPUS: 2500,
        }
    )
    
    _call_history: list[ModelCallResult] = PrivateAttr(default_factory=list)

    async def call(
        self,
        params: ModelCallParams,
        use_cache: bool = True,
    ) -> ModelCallResult:
        """
        Call a language model with deterministic seeding and caching.

        Args:
            params: Model call parameters
            use_cache: Whether to use cache for this call

        Returns:
            ModelCallResult with output, cost, latency, and provenance

        Raises:
            ValueError: If model is not allowlisted or params invalid
        """
        # Validate model
        if params.model not in self.cost_profiles:
            raise ValueError(f"Model {params.model} not allowlisted")

        # Compute cache key: (model, prompt_hash, seed)
        prompt_hash = hashlib.sha256(params.prompt.encode()).hexdigest()
        cache_key = f"{params.model.value}|{prompt_hash}|{params.seed}"

        # Check cache
        if use_cache:
            cached_output = self.cache.get(cache_key)
            if cached_output is not None:
                # Reconstruct result from cache
                result = ModelCallResult(
                    output=cached_output,
                    cost_usd=0.0,  # Cache hit = no cost
                    latency_ms=10.0,  # Cache lookup is fast
                    model=params.model,
                    seed=params.seed,
                    prompt_hash=prompt_hash,
                    output_hash=hashlib.sha256(cached_output.encode()).hexdigest(),
                    cached=True,
                )
                self._call_history.append(result)
                logger.info(f"Model call cached: {params.model.value} (key={cache_key[:16]}...)")
                return result

        # Simulate model call (stub - real implementation would call an actual API)
        start_time = time.time()
        simulated_output = f"Generated response from {params.model.value} with seed={params.seed}"
        latency_ms = (time.time() - start_time) * 1000 + self.latency_profiles.get(params.model, 1000)

        # Compute cost
        # Simplified: estimate token counts from prompt/output
        input_tokens = len(params.prompt) // 4  # Rough estimate
        output_tokens = len(simulated_output) // 4
        input_cost_per_1m, output_cost_per_1m = self.cost_profiles[params.model]
        cost_usd = (input_tokens * input_cost_per_1m + output_tokens * output_cost_per_1m) / 1_000_000

        output_hash = hashlib.sha256(simulated_output.encode()).hexdigest()

        # Cache result
        self.cache.put(cache_key, simulated_output)

        result = ModelCallResult(
            output=simulated_output,
            cost_usd=cost_usd,
            latency_ms=latency_ms,
            model=params.model,
            seed=params.seed,
            prompt_hash=prompt_hash,
            output_hash=output_hash,
            cached=False,
        )

        self._call_history.append(result)
        logger.info(
            f"Model call executed: {params.model.value} "
            f"(latency={latency_ms:.0f}ms, cost=${cost_usd:.6f}, seed={params.seed})"
        )

        return result

    def get_call_history(self) -> list[ModelCallResult]:
        """Retrieve all model calls made by this client."""
        return self._call_history.copy()

    def get_cache_stats(self) -> dict[str, Any]:
        """Get cache performance statistics."""
        return self.cache.stats()

    def clear_cache(self) -> None:
        """Clear the cache."""
        self.cache.clear()

    def validate_params(self, params: ModelCallParams) -> tuple[bool, str]:
        """
        Validate model call parameters.

        Returns:
            (is_valid, error_message)
        """
        if params.model not in self.cost_profiles:
            return False, f"Model {params.model} not allowlisted"

        if params.temperature < 0.0 or params.temperature > 2.0:
            return False, f"Temperature must be in [0.0, 2.0], got {params.temperature}"

        if params.max_tokens < 1 or params.max_tokens > 4096:
            return False, f"max_tokens must be in [1, 4096], got {params.max_tokens}"

        if params.top_p < 0.0 or params.top_p > 1.0:
            return False, f"top_p must be in [0.0, 1.0], got {params.top_p}"

        return True, ""
