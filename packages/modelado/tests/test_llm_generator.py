"""Tests for LLM-based function generator (Phase 9.3).

Tests:
- LLM generation with deterministic seed
- Token tracking and cost calculation
- Content-based caching
- Syntax validation
- Error handling
"""

import pytest
from unittest.mock import AsyncMock, MagicMock, patch
from datetime import datetime
from types import SimpleNamespace

from modelado.core.function_generators.llm_generator import (
    LLMFunctionGenerator,
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
def mock_openai_client():
    """Mock unified llm client."""
    client = AsyncMock()
    client.generate = AsyncMock()
    return client


@pytest.fixture
def sample_context():
    """Sample generation context."""
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


@pytest.mark.asyncio
async def test_llm_generator_initialization():
    """Test LLMFunctionGenerator initialization."""
    generator = LLMFunctionGenerator(api_key="sk-test")
    
    assert generator.name == "LLMFunctionGenerator"
    assert generator.strategy == GenerationStrategy.LLM_BASED
    assert generator.model == "gpt-4o-mini"
    assert generator.max_tokens == 1000
    assert generator.temperature == 0.0
    assert generator.enable_cache is True
    assert generator.generation_count == 0
    assert generator.total_cost_usd == 0.0


@pytest.mark.asyncio
async def test_llm_generation_with_valid_response(mock_openai_client, sample_context):
    """Test LLM generation with valid response."""
    # Mock LLM response
    mock_response = SimpleNamespace(
        text="""def sensitivity_analysis(context, parameters):
    base = context.get('revenue', 100.0)
    delta = parameters.get('delta_percent', 10.0)
    new_value = base * (1 + delta / 100.0)
    return {'status': 'ok', 'new_value': new_value}
""",
        usage={"prompt_tokens": 150, "completion_tokens": 80},
    )
    mock_openai_client.generate.return_value = mock_response
    
    # Create generator with mock client
    generator = LLMFunctionGenerator(api_key="sk-test", ai_client=mock_openai_client)
    
    # Generate
    operation = await generator.generate(sample_context)
    
    # Verify operation
    assert operation is not None
    assert operation.generated_function.language == "python"
    assert "sensitivity_analysis" in operation.generated_function.code
    assert operation.generated_function.generation_strategy == GenerationStrategy.LLM_BASED
    
    # Verify metrics
    stats = generator.get_stats()
    assert stats["generation_count"] == 1
    assert stats["total_cost_usd"] > 0.0
    assert stats["cache_hit_rate"] == 0.0


@pytest.mark.asyncio
async def test_llm_generation_with_seed_determinism(mock_openai_client, sample_context):
    """Test LLM generation uses seed for determinism."""
    mock_response = SimpleNamespace(text="def test(): return 42", usage={"prompt_tokens": 50, "completion_tokens": 20})
    mock_openai_client.generate.return_value = mock_response
    
    generator = LLMFunctionGenerator(api_key="sk-test", ai_client=mock_openai_client)
    
    await generator.generate(sample_context)
    
    # Verify seed was passed to LLM
    request = mock_openai_client.generate.call_args.args[0]
    assert request.seed == sample_context.seed


@pytest.mark.asyncio
async def test_llm_generation_caching(mock_openai_client, sample_context):
    """Test LLM generation uses cache for repeated requests."""
    mock_response = SimpleNamespace(text="def test(): return 42", usage={"prompt_tokens": 50, "completion_tokens": 20})
    mock_openai_client.generate.return_value = mock_response
    
    generator = LLMFunctionGenerator(api_key="sk-test", enable_cache=True, ai_client=mock_openai_client)
    
    # First generation (cache miss)
    op1 = await generator.generate(sample_context)
    assert mock_openai_client.generate.call_count == 1
    
    # Second generation (cache hit)
    op2 = await generator.generate(sample_context)
    assert mock_openai_client.generate.call_count == 1  # Not called again
    
    # Verify metrics
    stats = generator.get_stats()
    assert stats["generation_count"] == 2
    assert stats["cache_hits"] == 1
    assert stats["cache_hit_rate"] == 0.5


@pytest.mark.asyncio
async def test_llm_generation_syntax_error(mock_openai_client, sample_context):
    """Test LLM generation handles syntax errors."""
    # Mock LLM response with invalid Python
    mock_response = SimpleNamespace(text="def invalid syntax here", usage={"prompt_tokens": 50, "completion_tokens": 20})
    mock_openai_client.generate.return_value = mock_response
    
    generator = LLMFunctionGenerator(api_key="sk-test", ai_client=mock_openai_client)
    
    # Should raise GenerationError
    with pytest.raises(GenerationError, match="syntax error"):
        await generator.generate(sample_context)


@pytest.mark.asyncio
async def test_llm_cost_calculation():
    """Test LLM cost calculation."""
    generator = LLMFunctionGenerator(api_key="sk-test")
    
    # Test cost calculation
    cost = generator._compute_cost(input_tokens=1000, output_tokens=500)
    
    expected_cost = (
        (1000 / 1_000_000) * generator.INPUT_TOKEN_COST
        + (500 / 1_000_000) * generator.OUTPUT_TOKEN_COST
    )
    assert abs(cost - expected_cost) < 1e-9


@pytest.mark.asyncio
async def test_llm_cache_key_computation(sample_context):
    """Test cache key computation."""
    generator = LLMFunctionGenerator(api_key="sk-test")
    
    # Same context should produce same cache key
    key1 = generator._compute_cache_key(sample_context)
    key2 = generator._compute_cache_key(sample_context)
    assert key1 == key2
    
    # Different intent should produce different cache key
    context2 = create_test_context(
        user_instruction="Different instruction",
        intent_type="waterfall_analysis",
        semantic_features={},
        intent_confidence=0.85,
    )
    key3 = generator._compute_cache_key(context2)
    assert key1 != key3
