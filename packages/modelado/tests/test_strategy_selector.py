"""Tests for strategy selection logic (Phase 9.3).

Tests:
- Complexity scoring
- Strategy selection based on complexity and budgets
- Fallback chain when strategies fail
- Aggregated stats from sub-generators
- Cost-aware vs latency-aware selection
"""

import pytest
from unittest.mock import AsyncMock, MagicMock, patch

from modelado.core.function_generators.strategy_selector import (
    StrategySelector,
    COMPLEXITY_WEIGHTS,
)
from modelado.core.function_generators.base import (
    GenerationContext,
    GenerationError,
)
from modelado.core.generative_contracts import (
    GenerativeCommand,
    GenerationStrategy,
)
from test_helpers_generators import create_test_context


@pytest.fixture
def mock_api_key():
    """Mock OpenAI API key."""
    return "sk-test-key"


@pytest.fixture
def simple_context():
    """Simple sensitivity analysis context (low complexity)."""
    context = create_test_context(user_instruction="...", intent_type="...", semantic_features={}); command = context.command
    return GenerationContext(
        command=command,
        semantic_features={"revenue_detected": True},
        intent_type="sensitivity_analysis",
        intent_confidence=0.91,
        cost_budget_usd=0.01,
        latency_budget_ms=1000,
        seed=42,
    )


@pytest.fixture
def complex_context():
    """Complex context with multiple parameters and conditionals (high complexity)."""
    context = create_test_context(
        user_instruction="If revenue is 10 and costs are 20, compute the sum total and average.",
        intent_type="custom_analysis",
        semantic_features={},
    )
    command = context.command
    return GenerationContext(
        command=command,
        semantic_features={"revenue_detected": True, "cost_detected": True},
        intent_type="custom_analysis",
        intent_confidence=0.75,
        cost_budget_usd=0.01,
        latency_budget_ms=1000,
        seed=42,
    )


@pytest.mark.asyncio
async def test_strategy_selector_initialization(mock_api_key):
    """Test StrategySelector initialization."""
    selector = StrategySelector(api_key=mock_api_key, prefer_low_cost=True)
    
    assert selector.name == "StrategySelector"
    # StrategySelector is itself a generator; its primary strategy is LLM-based
    # (it may still choose lower-cost sub-strategies at runtime).
    assert selector.strategy == GenerationStrategy.LLM_BASED
    assert selector.composable is not None
    assert selector.template is not None
    assert selector.llm is not None
    assert selector.prefer_low_cost is True


@pytest.mark.asyncio
async def test_strategy_selector_without_llm():
    """Test StrategySelector without LLM (no API key)."""
    selector = StrategySelector(api_key=None)
    
    assert selector.composable is not None
    assert selector.template is not None
    assert selector.llm is None


@pytest.mark.asyncio
async def test_complexity_scoring_simple(simple_context):
    """Test complexity scoring for simple intent."""
    selector = StrategySelector(api_key="sk-test")
    
    complexity = selector._compute_complexity(simple_context)
    
    # Simple sensitivity analysis: should have low complexity
    # Expected: simple_arithmetic (1) + aggregation (0) = 1
    assert complexity <= 5


@pytest.mark.asyncio
async def test_complexity_scoring_complex(complex_context):
    """Test complexity scoring for complex intent."""
    selector = StrategySelector(api_key="sk-test")
    
    complexity = selector._compute_complexity(complex_context)
    
    # Complex with conditionals, aggregations, multi-parameter: should have high complexity
    # Expected: novel_intent (10) + multi_parameter (5) + conditional (5) + aggregation (2) = 22
    assert complexity >= 15


@pytest.mark.asyncio
async def test_strategy_selection_prefer_low_cost_simple(simple_context):
    """Test strategy selection prefers low-cost for simple intent."""
    selector = StrategySelector(api_key="sk-test", prefer_low_cost=True)
    
    complexity = selector._compute_complexity(simple_context)
    strategy = selector._select_strategy(simple_context, complexity)
    
    # Simple intent with known template: should prefer composable or template
    assert strategy in [
        GenerationStrategy.COMPOSABLE_BUILDING_BLOCKS,
        GenerationStrategy.TEMPLATE_INJECTION,
    ]


@pytest.mark.asyncio
async def test_strategy_selection_prefer_low_cost_complex(complex_context):
    """Test strategy selection uses LLM for complex intent."""
    selector = StrategySelector(api_key="sk-test", prefer_low_cost=True)
    
    complexity = selector._compute_complexity(complex_context)
    strategy = selector._select_strategy(complex_context, complexity)
    
    # Complex intent: should fall back to LLM (if budget allows)
    assert strategy == GenerationStrategy.LLM_BASED


@pytest.mark.asyncio
async def test_strategy_selection_latency_aware():
    """Test latency-aware strategy selection."""
    # Very tight latency budget: should prefer composable
    context = create_test_context(user_instruction="...", intent_type="...", semantic_features={}); command = context.command
    context = GenerationContext(
        command=command,
        semantic_features={},
        intent_type="arithmetic",
        intent_confidence=0.99,
        cost_budget_usd=0.01,
        latency_budget_ms=10,  # Very tight
    )
    
    selector = StrategySelector(api_key="sk-test", prefer_low_cost=False)
    complexity = selector._compute_complexity(context)
    strategy = selector._select_strategy(context, complexity)
    
    # Tight latency budget: should prefer composable (<1ms)
    assert strategy == GenerationStrategy.COMPOSABLE_BUILDING_BLOCKS


@pytest.mark.asyncio
async def test_fallback_chain_composable_first():
    """Test fallback chain when composable is primary."""
    selector = StrategySelector(api_key="sk-test")
    
    chain = selector._get_fallback_chain(GenerationStrategy.COMPOSABLE_BUILDING_BLOCKS)
    
    # Expected: composable → template → llm
    assert chain[0] == GenerationStrategy.COMPOSABLE_BUILDING_BLOCKS
    assert GenerationStrategy.TEMPLATE_INJECTION in chain
    if selector.llm:
        assert GenerationStrategy.LLM_BASED in chain


@pytest.mark.asyncio
async def test_fallback_chain_template_first():
    """Test fallback chain when template is primary."""
    selector = StrategySelector(api_key="sk-test")
    
    chain = selector._get_fallback_chain(GenerationStrategy.TEMPLATE_INJECTION)
    
    # Expected: template → composable → llm
    assert chain[0] == GenerationStrategy.TEMPLATE_INJECTION
    assert GenerationStrategy.COMPOSABLE_BUILDING_BLOCKS in chain


@pytest.mark.asyncio
async def test_fallback_chain_llm_first():
    """Test fallback chain when LLM is primary."""
    selector = StrategySelector(api_key="sk-test")
    
    chain = selector._get_fallback_chain(GenerationStrategy.LLM_BASED)
    
    # Expected: llm → template → composable
    if selector.llm:
        assert chain[0] == GenerationStrategy.LLM_BASED
    assert GenerationStrategy.TEMPLATE_INJECTION in chain


@pytest.mark.asyncio
async def test_strategy_execution_composable(simple_context):
    """Test strategy execution with composable builder."""
    selector = StrategySelector(api_key=None)  # No LLM
    
    operation = await selector.generate(simple_context)
    
    # Should use composable or template (zero cost)
    assert operation is not None
    assert operation.generated_function.generation_strategy in [
        GenerationStrategy.COMPOSABLE_BUILDING_BLOCKS,
        GenerationStrategy.TEMPLATE_INJECTION,
    ]
    
    stats = selector.get_stats()
    assert stats["total_cost_usd"] == 0.0


@pytest.mark.asyncio
async def test_aggregated_stats():
    """Test aggregated stats from all sub-generators."""
    selector = StrategySelector(api_key=None)
    
    # Generate with composable
    context = create_test_context(user_instruction="...", intent_type="...", semantic_features={}); command = context.command
    context = GenerationContext(
        command=command,
        semantic_features={},
        intent_type="sensitivity_analysis",
        intent_confidence=0.90,
    )
    
    await selector.generate(context)
    
    stats = selector.get_stats()
    
    # Verify aggregated stats
    assert "generation_count" in stats
    assert "total_cost_usd" in stats
    assert "sub_generators" in stats
    assert "composable" in stats["sub_generators"]
    assert "template" in stats["sub_generators"]


@pytest.mark.asyncio
async def test_all_strategies_fail_error():
    """Test error when all strategies fail."""
    selector = StrategySelector(api_key=None)
    
    # Mock all strategies to fail
    async def mock_fail(context):
        raise GenerationError("Mock failure")
    
    selector.composable.generate = mock_fail
    selector.template.generate = mock_fail
    
    context = create_test_context(user_instruction="...", intent_type="...", semantic_features={}); command = context.command
    context = GenerationContext(
        command=command,
        semantic_features={},
        intent_type="unknown",
        intent_confidence=0.5,
    )
    
    with pytest.raises(GenerationError, match="All strategies failed"):
        await selector.generate(context)
