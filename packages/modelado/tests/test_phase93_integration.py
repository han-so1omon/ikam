"""Integration tests for generator integration with operation handlers.

Validates that EconomicOperationHandler properly uses StrategySelector to generate
functions using composable/template/LLM strategies based on complexity and budgets.
"""

import pytest
from modelado.core.generative_contracts import GenerativeCommand
from modelado.core.operation_handlers import EconomicOperationHandler


@pytest.mark.asyncio
async def test_economic_handler_uses_phase93_generator():
    """Verify EconomicOperationHandler uses the generator (not hardcoded templates)."""
    handler = EconomicOperationHandler()
    
    # Create a simple economic command
    command = GenerativeCommand.create(
        user_instruction="Calculate sensitivity analysis for revenue",
        operation_type="economic_function",
        context={"project_id": "test"},
    )
    
    # Generate operation
    result = await handler.handle(command)
    
    # Verify result structure
    assert result is not None
    assert result.command_id == command.command_id
    assert result.generated_function is not None
    assert result.generated_function.code is not None
    assert result.generated_function.name is not None
    
    # Verify metadata includes a stable, semantic generator version
    assert result.generation_metadata.get("generator_version") == "semantic_operation_handlers_v1"
    assert "strategy" in result.generation_metadata
    assert "generation_cost_usd" in result.generation_metadata
    assert "generation_latency_ms" in result.generation_metadata
    
    # Verify strategy is one of the supported strategies
    strategy = result.generation_metadata["strategy"]
    assert strategy in [
        "llm_based",
        "composable_building_blocks",
        "template_injection",
        "unknown",
    ]


@pytest.mark.asyncio
async def test_handler_uses_zero_cost_strategies_by_default():
    """Verify handler prefers zero-cost strategies (composable/template) over LLM."""
    handler = EconomicOperationHandler()
    
    # Create simple command that should match composable blocks
    command = GenerativeCommand.create(
        user_instruction="Add revenue and costs",
        operation_type="economic_function",
        context={},
    )
    
    result = await handler.handle(command)
    
    # Should use zero-cost strategy (composable or template)
    assert result.generation_metadata.get("generation_cost_usd") == 0.0
    assert result.generation_metadata["strategy"] in [
        "composable_building_blocks",
        "template_injection",
    ]


@pytest.mark.asyncio
async def test_handler_stats_include_generator_metrics():
    """Verify handler stats include generator performance metrics."""
    handler = EconomicOperationHandler()
    
    # Process a command to populate stats
    command = GenerativeCommand.create(
        user_instruction="Calculate waterfall analysis",
        operation_type="economic_function",
        context={},
    )
    await handler.handle(command)
    
    # Get stats
    stats = handler.get_stats()
    
    # Verify generator stats are included
    assert "generator" in stats
    gen_stats = stats["generator"]
    assert "strategy" in gen_stats
    assert "generation_count" in gen_stats
    assert "total_cost_usd" in gen_stats
    assert "avg_latency_ms" in gen_stats
    assert "cache_hit_rate" in gen_stats


@pytest.mark.asyncio
async def test_handler_handles_multiple_commands():
    """Verify handler maintains correct stats across multiple commands."""
    handler = EconomicOperationHandler()
    
    commands = [
        "Calculate sensitivity analysis",
        "Generate waterfall decomposition",
        "Analyze unit economics",
    ]
    
    for instruction in commands:
        command = GenerativeCommand.create(
            user_instruction=instruction,
            operation_type="economic_function",
            context={},
        )
        result = await handler.handle(command)
        assert result is not None
        assert result.generation_metadata.get("generator_version") == "semantic_operation_handlers_v1"
    
    # Verify stats reflect all commands
    stats = handler.get_stats()
    assert stats["processed_count"] == 3
    assert stats["generator"]["generation_count"] >= 3  # May be higher due to fallbacks


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
