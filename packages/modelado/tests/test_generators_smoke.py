"""Smoke tests for Phase 9.3 function generators.

Quick validation that all generators initialize and can generate basic functions.
"""

import pytest
from test_helpers_generators import create_test_context

from modelado.core.function_generators.llm_generator import LLMFunctionGenerator
from modelado.core.function_generators.composable_builder import ComposableFunctionBuilder
from modelado.core.function_generators.template_injector import TemplateInjector
from modelado.core.function_generators.strategy_selector import StrategySelector
from modelado.core.generative_contracts import GenerationStrategy


@pytest.mark.asyncio
async def test_composable_builder_smoke():
    """Smoke test: ComposableFunctionBuilder can generate."""
    builder = ComposableFunctionBuilder()
    context = create_test_context(
        user_instruction="Increase revenue by 10%",
        intent_type="sensitivity_analysis",
        semantic_features={"revenue_detected": True},
    )
    
    operation = await builder.generate(context)
    
    assert operation is not None
    assert operation.generated_function is not None
    assert operation.generated_function.generation_strategy == GenerationStrategy.COMPOSABLE_BUILDING_BLOCKS
    assert builder.total_cost_usd == 0.0  # Zero cost


@pytest.mark.asyncio
async def test_template_injector_smoke():
    """Smoke test: TemplateInjector can generate."""
    injector = TemplateInjector()
    context = create_test_context(
        user_instruction="Increase revenue by 15%",
        intent_type="sensitivity_analysis",
        semantic_features={"revenue_detected": True},
    )
    
    operation = await injector.generate(context)
    
    assert operation is not None
    assert operation.generated_function is not None
    assert operation.generated_function.generation_strategy == GenerationStrategy.TEMPLATE_INJECTION
    assert "15" in operation.generated_function.code  # Extracted percentage
    assert injector.total_cost_usd == 0.0  # Zero cost


@pytest.mark.asyncio
async def test_strategy_selector_smoke():
    """Smoke test: StrategySelector can choose and execute strategy."""
    selector = StrategySelector(api_key=None)  # No LLM, will use composable/template
    context = create_test_context(
        user_instruction="Calculate waterfall breakdown",
        intent_type="waterfall_analysis",
        semantic_features={},
    )
    
    operation = await selector.generate(context)
    
    assert operation is not None
    assert operation.generated_function is not None
    # Should use zero-cost strategy (composable or template)
    assert operation.generated_function.generation_strategy in [
        GenerationStrategy.COMPOSABLE_BUILDING_BLOCKS,
        GenerationStrategy.TEMPLATE_INJECTION,
    ]


@pytest.mark.asyncio
async def test_all_generators_have_stats():
    """Smoke test: All generators provide stats."""
    composable = ComposableFunctionBuilder()
    template = TemplateInjector()
    selector = StrategySelector(api_key=None)
    
    # All should have stats method
    assert composable.get_stats() is not None
    assert template.get_stats() is not None
    assert selector.get_stats() is not None
    
    # Stats should include key metrics
    for generator in [composable, template, selector]:
        stats = generator.get_stats()
        assert "generation_count" in stats
        assert "total_cost_usd" in stats
        assert "cache_hit_rate" in stats
