"""Base interface for function generators.

All generation strategies (LLM, composable, template) implement the FunctionGenerator protocol.
"""

from __future__ import annotations

from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from typing import Any, Dict, Optional
from datetime import datetime

from modelado.core.generative_contracts import GenerativeCommand, GeneratedOperation, GenerationStrategy


@dataclass
class GenerationContext:
    """Context passed to generators with semantic features and constraints.
    
    Attributes:
        command: User command with intent and operation type
        semantic_features: Extracted features from semantic analysis
        intent_type: Classified intent (e.g., "sensitivity_analysis")
        intent_confidence: Confidence score (0.0-1.0)
        strategy_hint: Preferred generation strategy (optional)
        cost_budget_usd: Maximum cost allowed for this generation (optional)
        latency_budget_ms: Maximum latency allowed (optional)
        seed: Deterministic seed for reproducible generation (optional)
        model_version: LLM or generator version for provenance
        cache_key: Content hash for cache lookup (optional)
    """
    command: GenerativeCommand
    semantic_features: Dict[str, Any]
    intent_type: str
    intent_confidence: float
    strategy_hint: Optional[GenerationStrategy] = None
    cost_budget_usd: Optional[float] = None
    latency_budget_ms: Optional[float] = None
    seed: Optional[int] = None
    model_version: Optional[str] = None
    cache_key: Optional[str] = None
    metadata: Dict[str, Any] = field(default_factory=dict)


class FunctionGenerator(ABC):
    """Abstract base for all function generation strategies.
    
    Implementers:
    - LLMFunctionGenerator (GPT-4o-mini with seed)
    - ComposableFunctionBuilder (compose atomic operations)
    - TemplateInjector (enhanced templates)
    
    All generators must:
    1. Accept GenerationContext
    2. Return GeneratedOperation with full provenance
    3. Be deterministic (same context + seed → same output)
    4. Record cost and latency metrics
    """
    
    def __init__(self, name: str, strategy: GenerationStrategy):
        """Initialize generator.
        
        Args:
            name: Human-readable generator name (e.g., "GPT4oMiniGenerator")
            strategy: Generation strategy enum
        """
        self.name = name
        self.strategy = strategy
        self.generation_count = 0
        self.total_cost_usd = 0.0
        self.total_latency_ms = 0.0
        self.cache_hits = 0
        self.cache_misses = 0
    
    @abstractmethod
    async def generate(self, context: GenerationContext) -> GeneratedOperation:
        """Generate executable function from context.
        
        Must be deterministic: same context + seed → same output.
        
        Args:
            context: Generation context with command, features, constraints
        
        Returns:
            GeneratedOperation with generated function and metadata
        
        Raises:
            GenerationError: If generation fails
        """
        pass
    
    def get_stats(self) -> Dict[str, Any]:
        """Return generation statistics.
        
        Returns:
            Dict with counts, cost, latency, cache metrics
        """
        avg_cost = self.total_cost_usd / self.generation_count if self.generation_count > 0 else 0.0
        avg_latency = self.total_latency_ms / self.generation_count if self.generation_count > 0 else 0.0
        cache_hit_rate = self.cache_hits / (self.cache_hits + self.cache_misses) if (self.cache_hits + self.cache_misses) > 0 else 0.0
        
        return {
            "generator_name": self.name,
            "strategy": self.strategy.value,
            "generation_count": self.generation_count,
            "total_cost_usd": self.total_cost_usd,
            "avg_cost_usd": avg_cost,
            "total_latency_ms": self.total_latency_ms,
            "avg_latency_ms": avg_latency,
            "cache_hits": self.cache_hits,
            "cache_misses": self.cache_misses,
            "cache_hit_rate": cache_hit_rate,
        }
    
    def _record_generation(self, cost_usd: float, latency_ms: float, cache_hit: bool) -> None:
        """Record metrics for a generation.
        
        Args:
            cost_usd: Cost in USD for this generation
            latency_ms: Latency in milliseconds
            cache_hit: Whether result was from cache
        """
        self.generation_count += 1
        self.total_cost_usd += cost_usd
        self.total_latency_ms += latency_ms
        if cache_hit:
            self.cache_hits += 1
        else:
            self.cache_misses += 1


class GenerationError(Exception):
    """Raised when function generation fails."""
    pass
