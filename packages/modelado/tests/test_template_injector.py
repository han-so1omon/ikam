"""Tests for template injection with parameter extraction (Phase 9.3).

Tests:
- Parameter extraction from instruction text
- Template selection based on intent type
- Template filling with extracted parameters
- Fallback to defaults for missing parameters
- Syntax validation
- Caching
"""

import pytest

from modelado.core.function_generators.template_injector import (
    TemplateInjector,
    INTENT_TEMPLATES,
    PARAMETER_PATTERNS,
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
    context = create_test_context(
        user_instruction="Increase revenue by 15%",
        intent_type="sensitivity_analysis",
        semantic_features={"revenue_detected": True},
    )
    command = context.command
    return GenerationContext(
        command=command,
        semantic_features={"revenue_detected": True},
        intent_type="sensitivity_analysis",
        intent_confidence=0.91,
    )


@pytest.mark.asyncio
async def test_template_injector_initialization():
    """Test TemplateInjector initialization."""
    injector = TemplateInjector()
    
    assert injector.name == "TemplateInjector"
    assert injector.strategy == GenerationStrategy.TEMPLATE_INJECTION
    assert injector.enable_cache is True
    assert set(injector.templates.keys()) == {
        "sensitivity_analysis",
        "waterfall_analysis",
        "unit_economics_analysis",
        "generic_economic_operation",
    }


@pytest.mark.asyncio
async def test_template_injection_sensitivity_analysis(sample_context):
    """Test template injection for sensitivity analysis."""
    injector = TemplateInjector()
    
    operation = await injector.generate(sample_context)
    
    # Verify operation
    assert operation is not None
    assert operation.generated_function.language == "python"
    assert "sensitivity_analysis" in operation.generated_function.code
    assert "15" in operation.generated_function.code  # Extracted percentage
    assert "revenue" in operation.generated_function.code.lower()
    assert operation.generated_function.generation_strategy == GenerationStrategy.TEMPLATE_INJECTION


@pytest.mark.asyncio
async def test_template_injection_waterfall_analysis():
    """Test template injection for waterfall analysis."""
    context = create_test_context(user_instruction="...", intent_type="...", semantic_features={}); command = context.command
    context = GenerationContext(
        command=command,
        semantic_features={"cost_detected": True},
        intent_type="waterfall_analysis",
        intent_confidence=0.88,
    )
    
    injector = TemplateInjector()
    operation = await injector.generate(context)
    
    # Verify waterfall-specific code
    assert "waterfall_analysis" in operation.generated_function.code
    assert "components" in operation.generated_function.code


@pytest.mark.asyncio
async def test_parameter_extraction_percentage(sample_context):
    """Test extraction of percentage parameters."""
    injector = TemplateInjector()
    
    params = injector._extract_parameters(sample_context)
    
    # Should extract "15" from "Increase revenue by 15%"
    assert "delta_percent" in params
    assert params["delta_percent"] == 15.0


@pytest.mark.asyncio
async def test_parameter_extraction_currency():
    """Test extraction of currency parameters."""
    context = create_test_context(
        user_instruction="Increase revenue by $1,250.50",
        intent_type="sensitivity_analysis",
        semantic_features={"revenue_detected": True},
    )
    command = context.command
    context = GenerationContext(
        command=command,
        semantic_features={"revenue_detected": True},
        intent_type="sensitivity_analysis",
        intent_confidence=0.92,
    )
    
    injector = TemplateInjector()
    params = injector._extract_parameters(context)
    
    # Should extract currency value
    assert "currency_value" in params
    assert params["currency_value"] == 1250.50


@pytest.mark.asyncio
async def test_parameter_extraction_semantic_features():
    """Test parameter extraction from semantic features."""
    context = create_test_context(user_instruction="...", intent_type="...", semantic_features={}); command = context.command
    context = GenerationContext(
        command=command,
        semantic_features={"cost_detected": True},
        intent_type="sensitivity_analysis",
        intent_confidence=0.90,
    )
    
    injector = TemplateInjector()
    params = injector._extract_parameters(context)
    
    # Should detect "cost" from semantic features
    assert "parameter_name" in params
    assert params["parameter_name"] == "cost"


@pytest.mark.asyncio
async def test_template_selection(sample_context):
    """Test template selection based on intent type."""
    injector = TemplateInjector()
    
    # Sensitivity analysis template
    template = injector._select_template(sample_context)
    assert template is not None
    assert "sensitivity_analysis" in template
    
    # Unknown intent type
    unknown_context = GenerationContext(
        command=GenerativeCommand(
            command_id="test-cmd-005",
            user_instruction="Unknown operation",
            operation_type="economic_function",
            context={},
        ),
        semantic_features={},
        intent_type="unknown_type",
        intent_confidence=0.5,
    )
    template = injector._select_template(unknown_context)
    assert template is None


@pytest.mark.asyncio
async def test_template_filling_with_extracted_params():
    """Test template filling with extracted parameters."""
    injector = TemplateInjector()
    template = INTENT_TEMPLATES["sensitivity_analysis"]
    
    context = create_test_context(user_instruction="...", intent_type="...", semantic_features={}); command = context.command
    context = GenerationContext(
        command=command,
        semantic_features={"revenue_detected": True},
        intent_type="sensitivity_analysis",
        intent_confidence=0.93,
    )
    
    params = {"delta_percent": 20.0, "parameter_name": "revenue"}
    filled = injector._fill_template(template, context, params)
    
    # Verify filled template
    assert "20" in filled  # Extracted percentage
    assert "revenue" in filled  # Extracted parameter name


@pytest.mark.asyncio
async def test_template_filling_fallback_defaults():
    """Test template filling uses defaults for missing parameters."""
    injector = TemplateInjector()
    template = INTENT_TEMPLATES["sensitivity_analysis"]
    
    context = create_test_context(user_instruction="...", intent_type="...", semantic_features={}); command = context.command
    context = GenerationContext(
        command=command,
        semantic_features={},
        intent_type="sensitivity_analysis",
        intent_confidence=0.85,
    )
    
    params = {}  # No extracted parameters
    filled = injector._fill_template(template, context, params)
    
    # Should use defaults
    assert "10.0" in filled  # Default delta_percent
    assert "value" in filled  # Default parameter_name


@pytest.mark.asyncio
async def test_template_syntax_validation():
    """Test syntax validation of filled templates."""
    injector = TemplateInjector()
    
    # Valid template
    valid_code = "def test(): return 42"
    injector._validate_filled_template(valid_code)  # Should not raise
    
    # Invalid template
    invalid_code = "def invalid syntax here"
    with pytest.raises(GenerationError, match="syntax error"):
        injector._validate_filled_template(invalid_code)


@pytest.mark.asyncio
async def test_template_caching(sample_context):
    """Test caching in template injector."""
    injector = TemplateInjector(enable_cache=True)
    
    # First generation (cache miss)
    op1 = await injector.generate(sample_context)
    
    # Second generation (cache hit)
    op2 = await injector.generate(sample_context)
    
    # Verify metrics
    stats = injector.get_stats()
    assert stats["generation_count"] == 2
    assert stats["cache_hits"] == 1
    assert stats["cache_hit_rate"] == 0.5


@pytest.mark.asyncio
async def test_template_zero_cost():
    """Test that template injection has zero cost."""
    injector = TemplateInjector()
    
    context = create_test_context(user_instruction="...", intent_type="...", semantic_features={}); command = context.command
    context = GenerationContext(
        command=command,
        semantic_features={},
        intent_type="unit_economics_analysis",
        intent_confidence=0.94,
    )
    
    await injector.generate(context)
    
    stats = injector.get_stats()
    assert stats["total_cost_usd"] == 0.0


@pytest.mark.asyncio
async def test_template_unavailable_error(sample_context):
    """Test error handling when template unavailable."""
    injector = TemplateInjector()
    
    # Remove template
    injector.templates = {}
    
    with pytest.raises(GenerationError, match="No template available"):
        await injector.generate(sample_context)
