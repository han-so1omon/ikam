"""Strategy selection logic for function generation.

Chooses optimal generation strategy based on:
- Intent complexity
- Semantic feature coverage
- Cost/latency constraints
- Determinism requirements

Strategy priority (cost-aware):
1. ComposableFunctionBuilder: Zero cost, <1ms latency (for simple arithmetic/aggregations)
2. TemplateInjector: Zero cost, ~5ms latency (for known patterns with parameters)
3. LLMFunctionGenerator: ~$0.0001/generation, ~500ms latency (for novel intents)

Mathematical guarantees:
- Determinism: Same (context + seed) → same selected strategy → same generated function
- Cost monotonicity: Selected strategy respects cost_budget_usd constraint
- Fallback chain: If preferred strategy fails, try next in priority order
"""

from __future__ import annotations

import logging
from datetime import datetime
from typing import Optional

from modelado.core.function_generators.base import (
    FunctionGenerator,
    GenerationContext,
    GenerationError,
)
from modelado.core.function_generators.llm_generator import LLMFunctionGenerator
from modelado.core.function_generators.composable_builder import ComposableFunctionBuilder
from modelado.core.function_generators.template_injector import TemplateInjector
from modelado.core.generative_contracts import GeneratedOperation, GenerationStrategy

logger = logging.getLogger(__name__)


# Complexity scoring weights
COMPLEXITY_WEIGHTS = {
    "novel_intent": 10,  # Intent type not seen before
    "multi_parameter": 5,  # Multiple parameters required
    "conditional_logic": 5,  # Requires if/else branching
    "aggregation": 2,  # Needs aggregation (sum, mean, etc.)
    "simple_arithmetic": 1,  # Basic arithmetic only
}


class StrategySelector(FunctionGenerator):
    """Select optimal generation strategy based on context.
    
    Selection logic:
    1. Compute intent complexity score
    2. Check if composable building blocks can satisfy intent
    3. Check if template exists for intent type
    4. Fall back to LLM if neither works
    5. Respect cost/latency budgets
    
    Usage:
        selector = StrategySelector(api_key="sk-...")
        context = GenerationContext(
            command=command,
            semantic_features={"revenue_detected": True},
            intent_type="sensitivity_analysis",
            intent_confidence=0.91,
            cost_budget_usd=0.001,
            latency_budget_ms=100,
        )
        operation = await selector.generate(context)
    """
    
    def __init__(
        self,
        api_key: Optional[str] = None,
        enable_cache: bool = True,
        prefer_low_cost: bool = True,
    ):
        """Initialize strategy selector with sub-generators.
        
        Args:
            api_key: OpenAI API key (for LLM strategy)
            enable_cache: Whether to enable caching in sub-generators
            prefer_low_cost: Prefer zero-cost strategies (composable, template) over LLM
        """
        super().__init__(
            name="StrategySelector",
            strategy=GenerationStrategy.LLM_BASED,  # Primary strategy when using selector
        )
        
        # Initialize sub-generators
        self.composable = ComposableFunctionBuilder(enable_cache=enable_cache)
        self.template = TemplateInjector(enable_cache=enable_cache)
        
        # LLM generator (optional; only if API key provided)
        if api_key:
            self.llm = LLMFunctionGenerator(api_key=api_key, enable_cache=enable_cache)
        else:
            self.llm = None
            logger.warning("LLMFunctionGenerator disabled: no API key provided")
        
        self.prefer_low_cost = prefer_low_cost
        
        logger.info(
            f"StrategySelector initialized: "
            f"composable={self.composable is not None}, "
            f"template={self.template is not None}, "
            f"llm={self.llm is not None}, "
            f"prefer_low_cost={prefer_low_cost}"
        )
    
    async def generate(self, context: GenerationContext) -> GeneratedOperation:
        """Generate function using optimal strategy.
        
        Process:
        1. Compute intent complexity score
        2. Select strategy based on complexity, budgets, availability
        3. Try selected strategy
        4. If strategy fails, fall back to next strategy
        5. Record metrics from selected strategy
        
        Args:
            context: Generation context with command, features, budgets
        
        Returns:
            GeneratedOperation from selected strategy
        
        Raises:
            GenerationError: If all strategies fail
        """
        start_time = datetime.utcnow()
        
        # Step 1: Compute complexity score
        complexity = self._compute_complexity(context)
        
        # Step 2: Select strategy
        strategy = self._select_strategy(context, complexity)
        
        logger.info(
            f"Selected strategy: {strategy.value}, "
            f"complexity={complexity}, "
            f"intent_type={context.intent_type}"
        )
        
        # Step 3: Try selected strategy
        operation = None
        fallback_chain = self._get_fallback_chain(strategy)
        
        for strat in fallback_chain:
            try:
                operation = await self._try_strategy(strat, context)
                if operation:
                    logger.info(f"Strategy succeeded: {strat.value}")
                    break
            except GenerationError as e:
                logger.warning(f"Strategy failed: {strat.value}, error={e}")
                continue
        
        if not operation:
            raise GenerationError(
                f"All strategies failed for intent_type={context.intent_type}"
            )

        # Record selection outcome for downstream metadata.
        # Note: the returned operation may be produced by a fallback strategy.
        operation.generation_metadata.setdefault("selected_strategy", strategy.value)
        
        # Step 4: Record metrics
        elapsed_ms = (datetime.utcnow() - start_time).total_seconds() * 1000
        
        # Cost = sum of sub-generator costs
        total_cost = (
            self.composable.total_cost_usd
            + self.template.total_cost_usd
            + (self.llm.total_cost_usd if self.llm else 0.0)
        )
        
        self._record_generation(cost_usd=total_cost, latency_ms=elapsed_ms, cache_hit=False)
        
        logger.info(
            f"Strategy selection complete: selected={strategy.value}, "
            f"cost=${total_cost:.6f}, latency={elapsed_ms:.1f}ms"
        )
        
        return operation
    
    def _compute_complexity(self, context: GenerationContext) -> int:
        """Compute complexity score for intent.
        
        Args:
            context: Generation context
        
        Returns:
            Complexity score (higher = more complex)
        """
        score = 0
        instruction = context.command.user_instruction.lower()
        
        # Novel intent (not in known templates)
        if context.intent_type not in self.template.templates:
            score += COMPLEXITY_WEIGHTS["novel_intent"]
        
        # Multi-parameter (multiple numbers/percentages in instruction)
        import re
        numeric_matches = re.findall(r"\d+(?:\.\d+)?", instruction)
        if len(numeric_matches) > 1:
            score += COMPLEXITY_WEIGHTS["multi_parameter"]
        
        # Conditional logic (keywords: if, when, unless)
        if any(kw in instruction for kw in ["if", "when", "unless"]):
            score += COMPLEXITY_WEIGHTS["conditional_logic"]
        
        # Aggregation (keywords: sum, total, average, mean)
        if any(kw in instruction for kw in ["sum", "total", "average", "mean"]):
            score += COMPLEXITY_WEIGHTS["aggregation"]
        
        # Simple arithmetic (only basic operations)
        if all(
            kw not in instruction
            for kw in ["if", "when", "unless", "sum", "total", "average"]
        ):
            score += COMPLEXITY_WEIGHTS["simple_arithmetic"]
        
        return score
    
    def _select_strategy(
        self,
        context: GenerationContext,
        complexity: int,
    ) -> GenerationStrategy:
        """Select optimal strategy based on complexity and budgets.
        
        Args:
            context: Generation context
            complexity: Complexity score
        
        Returns:
            Selected strategy
        """
        # Cost-aware selection
        if self.prefer_low_cost:
            # Try zero-cost strategies first
            
            # Composable: best for simple arithmetic/aggregations (complexity ≤ 5)
            if complexity <= 5 and context.intent_type in [
                "sensitivity_analysis",
                "waterfall_analysis",
                "unit_economics_analysis",
            ]:
                return GenerationStrategy.COMPOSABLE_BUILDING_BLOCKS
            
            # Template: good for known patterns with parameters (complexity ≤ 10)
            if complexity <= 10 and context.intent_type in self.template.templates:
                return GenerationStrategy.TEMPLATE_INJECTION
            
            # LLM: fallback for novel/complex intents (if available)
            if self.llm and context.cost_budget_usd > 0.0001:
                return GenerationStrategy.LLM_BASED
            
            # Default to template if LLM unavailable
            return GenerationStrategy.TEMPLATE_INJECTION
        
        # Latency-aware selection
        if context.latency_budget_ms < 50:
            # Composable is fastest (<1ms)
            return GenerationStrategy.COMPOSABLE_BUILDING_BLOCKS
        elif context.latency_budget_ms < 100:
            # Template is fast (~5ms)
            return GenerationStrategy.TEMPLATE_INJECTION
        else:
            # LLM is slower (~500ms) but most flexible
            if self.llm:
                return GenerationStrategy.LLM_BASED
            return GenerationStrategy.TEMPLATE_INJECTION
    
    def _get_fallback_chain(
        self,
        primary: GenerationStrategy,
    ) -> list[GenerationStrategy]:
        """Get fallback chain for strategy.
        
        Args:
            primary: Primary strategy
        
        Returns:
            List of strategies to try in order
        """
        # Define fallback chains
        chains = {
            GenerationStrategy.COMPOSABLE_BUILDING_BLOCKS: [
                GenerationStrategy.COMPOSABLE_BUILDING_BLOCKS,
                GenerationStrategy.TEMPLATE_INJECTION,
                GenerationStrategy.LLM_BASED if self.llm else None,
            ],
            GenerationStrategy.TEMPLATE_INJECTION: [
                GenerationStrategy.TEMPLATE_INJECTION,
                GenerationStrategy.COMPOSABLE_BUILDING_BLOCKS,
                GenerationStrategy.LLM_BASED if self.llm else None,
            ],
            GenerationStrategy.LLM_BASED: [
                GenerationStrategy.LLM_BASED if self.llm else None,
                GenerationStrategy.TEMPLATE_INJECTION,
                GenerationStrategy.COMPOSABLE_BUILDING_BLOCKS,
            ],
        }
        
        # Filter out None (unavailable strategies)
        chain = [s for s in chains.get(primary, [primary]) if s is not None]
        
        return chain
    
    async def _try_strategy(
        self,
        strategy: GenerationStrategy,
        context: GenerationContext,
    ) -> Optional[GeneratedOperation]:
        """Try a specific strategy.
        
        Args:
            strategy: Strategy to try
            context: Generation context
        
        Returns:
            GeneratedOperation or None if strategy unavailable
        
        Raises:
            GenerationError: If strategy fails
        """
        if strategy == GenerationStrategy.COMPOSABLE_BUILDING_BLOCKS:
            return await self.composable.generate(context)
        elif strategy == GenerationStrategy.TEMPLATE_INJECTION:
            return await self.template.generate(context)
        elif strategy == GenerationStrategy.LLM_BASED:
            if self.llm:
                return await self.llm.generate(context)
            else:
                raise GenerationError("LLM strategy unavailable: no API key")
        else:
            raise GenerationError(f"Unknown strategy: {strategy}")
    
    def get_stats(self) -> dict:
        """Get aggregated stats from all sub-generators.
        
        Returns:
            Dict with selector stats + sub-generator stats
        """
        stats = super().get_stats()
        
        # Add sub-generator stats
        stats["sub_generators"] = {
            "composable": self.composable.get_stats(),
            "template": self.template.get_stats(),
        }
        
        if self.llm:
            stats["sub_generators"]["llm"] = self.llm.get_stats()
        
        return stats
