"""Tests for composable building blocks generator.

Tests:
- Building block selection based on intent
- Zero-cost composition
- Block composition and code generation
- Syntax validation
- Caching
"""

import pytest
from datetime import datetime

from modelado.core.function_generators.composable_builder import (
    ComposableFunctionBuilder,
    BUILDING_BLOCKS,
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
def sample_context():
    """Sample generation context for sensitivity analysis."""
    context = create_test_context(user_instruction="...", intent_type="...", semantic_features={}); command = context.command
    return GenerationContext(
        command=command,
        semantic_features={"revenue_detected": True},
        intent_type="sensitivity_analysis",
        intent_confidence=0.91,
    )


@pytest.mark.asyncio
async def test_composable_builder_initialization():
    """Test ComposableFunctionBuilder initialization."""
    builder = ComposableFunctionBuilder()
    
    assert builder.name == "ComposableFunctionBuilder"
    assert builder.strategy == GenerationStrategy.COMPOSABLE_BUILDING_BLOCKS
    assert builder.enable_cache is True
    assert builder.generation_count == 0
    assert builder.total_cost_usd == 0.0
    assert len(BUILDING_BLOCKS) == 12


@pytest.mark.asyncio
async def test_composable_generation_sensitivity_analysis(sample_context):
    """Test composable generation for sensitivity analysis."""
    builder = ComposableFunctionBuilder()
    
    operation = await builder.generate(sample_context)
    
    # Verify operation
    assert operation is not None
    assert operation.generated_function.language == "python"
    assert "sensitivity" in operation.generated_function.code.lower()
    assert "percent_change" in operation.generated_function.code.lower()
    assert operation.generated_function.generation_strategy == GenerationStrategy.COMPOSABLE_BUILDING_BLOCKS
    
    # Verify zero cost
    stats = builder.get_stats()
    assert stats["total_cost_usd"] == 0.0
    assert stats["generation_count"] == 1


@pytest.mark.asyncio
async def test_composable_generation_waterfall_analysis():
    """Test composable generation for waterfall analysis."""
    context = create_test_context(user_instruction="...", intent_type="...", semantic_features={}); command = context.command
    context = GenerationContext(
        command=command,
        semantic_features={"revenue_detected": True, "breakdown_requested": True},
        intent_type="waterfall_analysis",
        intent_confidence=0.88,
    )
    
    builder = ComposableFunctionBuilder()
    operation = await builder.generate(context)
    
    # Verify waterfall-specific blocks
    assert "waterfall" in operation.generated_function.code.lower()
    assert operation.generated_function.generation_strategy == GenerationStrategy.COMPOSABLE_BUILDING_BLOCKS


@pytest.mark.asyncio
async def test_composable_building_block_selection(sample_context):
    """Test building block selection based on intent."""
    builder = ComposableFunctionBuilder()
    
    # Select blocks for sensitivity analysis
    blocks = builder._select_building_blocks(sample_context)
    
    # Should include sensitivity and percent_change
    assert "sensitivity" in blocks
    assert "percent_change" in blocks
    assert len(blocks) >= 2


@pytest.mark.asyncio
async def test_composable_block_composition():
    """Test block composition into Python function."""
    builder = ComposableFunctionBuilder()
    
    context = create_test_context(user_instruction="...", intent_type="...", semantic_features={}); command = context.command
    context = GenerationContext(
        command=command,
        semantic_features={"aggregation_requested": True},
        intent_type="aggregation",
        intent_confidence=0.95,
    )
    
    blocks = {"sum": BUILDING_BLOCKS["sum"], "mean": BUILDING_BLOCKS["mean"]}
    code = builder._compose_blocks(context, blocks)
    
    # Verify composed code
    assert "def generated_function" in code
    assert "sum" in code
    assert "mean" in code
    assert "return result" in code


@pytest.mark.asyncio
async def test_composable_syntax_validation():
    """Test syntax validation of composed blocks."""
    builder = ComposableFunctionBuilder()
    
    # Valid code
    valid_code = "def test(): return 42"
    builder._validate_composition(valid_code)  # Should not raise
    
    # Invalid code
    invalid_code = "def invalid syntax here"
    with pytest.raises(GenerationError, match="syntax error"):
        builder._validate_composition(invalid_code)


@pytest.mark.asyncio
async def test_composable_caching(sample_context):
    """Test caching in composable builder."""
    builder = ComposableFunctionBuilder(enable_cache=True)
    
    # First generation (cache miss)
    op1 = await builder.generate(sample_context)
    
    # Second generation (cache hit)
    op2 = await builder.generate(sample_context)
    
    # Verify metrics
    stats = builder.get_stats()
    assert stats["generation_count"] == 2
    assert stats["cache_hits"] == 1
    assert stats["cache_hit_rate"] == 0.5
    
    # Both operations should be identical
    assert op1.generated_function.code == op2.generated_function.code


@pytest.mark.asyncio
async def test_composable_zero_cost():
    """Test that composable generation has zero cost."""
    builder = ComposableFunctionBuilder()
    
    context = create_test_context(user_instruction="...", intent_type="...", semantic_features={}); command = context.command
    context = GenerationContext(
        command=command,
        semantic_features={},
        intent_type="arithmetic",
        intent_confidence=0.99,
    )
    
    await builder.generate(context)
    
    stats = builder.get_stats()
    assert stats["total_cost_usd"] == 0.0
    assert stats["avg_cost_usd"] == 0.0


@pytest.mark.asyncio
async def test_composable_low_latency():
    """Test that composable generation has low latency."""
    builder = ComposableFunctionBuilder()
    
    context = create_test_context(user_instruction="...", intent_type="...", semantic_features={}); command = context.command
    context = GenerationContext(
        command=command,
        semantic_features={},
        intent_type="arithmetic",
        intent_confidence=0.98,
    )
    
    start = datetime.utcnow()
    await builder.generate(context)
    elapsed_ms = (datetime.utcnow() - start).total_seconds() * 1000
    
    # Should complete in under 50ms
    assert elapsed_ms < 50.0


@pytest.mark.asyncio
async def test_composable_all_building_blocks():
    """Test that all 12 building blocks are available."""
    # Note: Implementation has 12 blocks, not 11 as originally planned
    assert len(BUILDING_BLOCKS) >= 11  # At least 11, may have more
    
    expected_blocks = {
        "add", "subtract", "multiply", "divide",
        "percent_change", "sum", "mean", "min", "max", "count",
        "sensitivity", "waterfall",
    }
    
    # All expected blocks should be present
    assert expected_blocks.issubset(set(BUILDING_BLOCKS.keys()))
    
    # Verify each block has lambda and type signature
    for name, (lambda_code, type_sig) in BUILDING_BLOCKS.items():
        assert lambda_code is not None
        assert type_sig is not None
        assert "lambda" in lambda_code
