"""
Unit tests for semantic_evaluators.py

Tests cover:
- All 5 semantic evaluators
- Evaluator registry
- Confidence scoring
- Feature detection
- Capability metadata
"""

import pytest

from modelado.semantic_evaluators import (
    EconomicFunctionEvaluator,
    StoryOperationEvaluator,
    ComparisonOperationEvaluator,
    ConstraintOperationEvaluator,
    CustomizationEvaluator,
    EvaluatorRegistry,
    create_default_registry,
    EvaluationResult,
)


class TestEconomicFunctionEvaluator:
    """Test EconomicFunctionEvaluator."""
    
    @pytest.fixture
    def evaluator(self):
        return EconomicFunctionEvaluator()
    
    @pytest.mark.asyncio
    async def test_evaluate_revenue_adjustment(self, evaluator):
        """Test evaluation of revenue adjustment intent."""
        intent = "Adjust Q4 revenue forecast by 15% increase"
        context = {}
        
        result = await evaluator.evaluate(intent, context)
        
        assert result.can_handle is True
        assert result.confidence >= 0.7
        assert result.semantic_features["involves_revenue"] is True
        assert result.semantic_features["needs_forecasting"] is True
        assert "revenue" in result.reasoning.lower() or "economic" in result.reasoning.lower()
    
    @pytest.mark.asyncio
    async def test_evaluate_sensitivity_analysis(self, evaluator):
        """Test evaluation of sensitivity analysis intent."""
        intent = "Run sensitivity analysis on all cost drivers"
        context = {}
        
        result = await evaluator.evaluate(intent, context)
        
        assert result.can_handle is True
        assert result.confidence >= 0.7
        assert result.semantic_features["involves_costs"] is True
        assert result.semantic_features["needs_sensitivity"] is True
    
    @pytest.mark.asyncio
    async def test_evaluate_correlation(self, evaluator):
        """Test evaluation of correlation intent."""
        intent = "Correlate revenue with market size using sigmoid curve"
        context = {}
        
        result = await evaluator.evaluate(intent, context)
        
        assert result.can_handle is True
        assert result.confidence >= 0.7
        assert result.semantic_features["involves_revenue"] is True
        assert result.semantic_features["needs_correlation"] is True
        assert result.semantic_features["uses_math_model"] is True
    
    @pytest.mark.asyncio
    async def test_evaluate_non_economic_intent(self, evaluator):
        """Test that non-economic intent is rejected."""
        intent = "Create a slide deck for investor presentation"
        context = {}
        
        result = await evaluator.evaluate(intent, context)
        
        assert result.can_handle is False
        assert result.confidence < 0.7
    
    def test_get_capabilities(self, evaluator):
        """Test getting evaluator capabilities."""
        caps = evaluator.get_capabilities()
        
        assert caps["name"] == "EconomicFunctionEvaluator"
        assert "supported_operations" in caps
        assert "revenue_adjustment" in caps["supported_operations"]


class TestStoryOperationEvaluator:
    """Test StoryOperationEvaluator."""
    
    @pytest.fixture
    def evaluator(self):
        return StoryOperationEvaluator()
    
    @pytest.mark.asyncio
    async def test_evaluate_narrative_generation(self, evaluator):
        """Test evaluation of narrative generation intent."""
        intent = "Generate narrative arc emphasizing unit economics improvement"
        context = {}
        
        result = await evaluator.evaluate(intent, context)
        
        assert result.can_handle is True
        assert result.confidence >= 0.7
        assert result.semantic_features["narrative_generation"] is True
    
    @pytest.mark.asyncio
    async def test_evaluate_slide_creation(self, evaluator):
        """Test evaluation of slide creation intent."""
        intent = "Create slide deck for Q4 investor update"
        context = {}
        
        result = await evaluator.evaluate(intent, context)
        
        assert result.can_handle is True
        assert result.confidence >= 0.7
        assert result.semantic_features["slide_creation"] is True
        assert result.semantic_features["investor_focus"] is True
    
    @pytest.mark.asyncio
    async def test_evaluate_theme_application(self, evaluator):
        """Test evaluation of theme application intent."""
        intent = "Apply minimalist theme to all presentation slides"
        context = {}
        
        result = await evaluator.evaluate(intent, context)
        
        assert result.can_handle is True
        assert result.confidence >= 0.7
        assert result.semantic_features["theme_application"] is True
        assert result.semantic_features["slide_creation"] is True
    
    @pytest.mark.asyncio
    async def test_evaluate_non_story_intent(self, evaluator):
        """Test that non-story intent is rejected."""
        intent = "Adjust revenue forecast by 15%"
        context = {}
        
        result = await evaluator.evaluate(intent, context)
        
        assert result.can_handle is False
        assert result.confidence < 0.7


class TestComparisonOperationEvaluator:
    """Test ComparisonOperationEvaluator."""
    
    @pytest.fixture
    def evaluator(self):
        return ComparisonOperationEvaluator()
    
    @pytest.mark.asyncio
    async def test_evaluate_comparison_intent(self, evaluator):
        """Test evaluation of comparison intent."""
        intent = "Compare three pricing strategies: penetration, premium, freemium"
        context = {}
        
        result = await evaluator.evaluate(intent, context)
        
        assert result.can_handle is True
        assert result.confidence >= 0.7
        assert result.semantic_features["involves_comparison"] is True
        assert result.semantic_features["multiple_options"] is True
    
    @pytest.mark.asyncio
    async def test_evaluate_tradeoff_analysis(self, evaluator):
        """Test evaluation of trade-off analysis intent."""
        intent = "Analyze trade-offs between growth and profitability"
        context = {}
        
        result = await evaluator.evaluate(intent, context)
        
        assert result.can_handle is True
        assert result.confidence >= 0.7
        assert result.semantic_features["trade_off_analysis"] is True
    
    @pytest.mark.asyncio
    async def test_evaluate_non_comparison_intent(self, evaluator):
        """Test that non-comparison intent is rejected."""
        intent = "Generate narrative for investor pitch"
        context = {}
        
        result = await evaluator.evaluate(intent, context)
        
        assert result.can_handle is False


class TestConstraintOperationEvaluator:
    """Test ConstraintOperationEvaluator."""
    
    @pytest.fixture
    def evaluator(self):
        return ConstraintOperationEvaluator()
    
    @pytest.mark.asyncio
    async def test_evaluate_optimization_intent(self, evaluator):
        """Test evaluation of optimization intent."""
        intent = "Optimize pricing to maximize revenue within market constraints"
        context = {}
        
        result = await evaluator.evaluate(intent, context)
        
        assert result.can_handle is True
        assert result.confidence >= 0.7
        assert result.semantic_features["needs_optimization"] is True
        assert result.semantic_features["has_constraints"] is True
    
    @pytest.mark.asyncio
    async def test_evaluate_bounded_operation(self, evaluator):
        """Test evaluation of bounded operation intent."""
        intent = "Keep costs within 20-25% range of revenue"
        context = {}
        
        result = await evaluator.evaluate(intent, context)
        
        assert result.can_handle is True
        assert result.confidence >= 0.7
        assert result.semantic_features["range_bounded"] is True
    
    @pytest.mark.asyncio
    async def test_evaluate_non_constraint_intent(self, evaluator):
        """Test that non-constraint intent is rejected."""
        intent = "Create slide deck"
        context = {}
        
        result = await evaluator.evaluate(intent, context)
        
        assert result.can_handle is False


class TestCustomizationEvaluator:
    """Test CustomizationEvaluator."""
    
    @pytest.fixture
    def evaluator(self):
        return CustomizationEvaluator()
    
    @pytest.mark.asyncio
    async def test_evaluate_styling_intent(self, evaluator):
        """Test evaluation of styling intent."""
        intent = "Apply professional theme with blue color scheme"
        context = {}
        
        result = await evaluator.evaluate(intent, context)
        
        assert result.can_handle is True
        assert result.confidence >= 0.7
        assert result.semantic_features["styling_operation"] is True
        assert result.semantic_features["theme_operation"] is True
    
    @pytest.mark.asyncio
    async def test_evaluate_format_intent(self, evaluator):
        """Test evaluation of format intent."""
        intent = "Configure export format to Excel with formulas"
        context = {}
        
        result = await evaluator.evaluate(intent, context)
        
        assert result.can_handle is True
        assert result.confidence >= 0.7
        assert result.semantic_features["format_operation"] is True
        assert result.semantic_features["configuration"] is True
    
    @pytest.mark.asyncio
    async def test_evaluate_non_customization_intent(self, evaluator):
        """Test that non-customization intent is rejected."""
        intent = "Run sensitivity analysis on costs"
        context = {}
        
        result = await evaluator.evaluate(intent, context)
        
        assert result.can_handle is False


class TestEvaluatorRegistry:
    """Test EvaluatorRegistry."""
    
    @pytest.fixture
    def registry(self):
        return EvaluatorRegistry()
    
    def test_register_evaluator(self, registry):
        """Test registering an evaluator."""
        evaluator = EconomicFunctionEvaluator()
        
        registry.register(evaluator)
        
        assert len(registry.get_all()) == 1
        assert registry.get_by_name("EconomicFunctionEvaluator") is evaluator
    
    def test_get_all_evaluators(self, registry):
        """Test getting all evaluators."""
        registry.register(EconomicFunctionEvaluator())
        registry.register(StoryOperationEvaluator())
        
        all_evaluators = registry.get_all()
        
        assert len(all_evaluators) == 2
    
    def test_get_by_name(self, registry):
        """Test getting evaluator by name."""
        evaluator = EconomicFunctionEvaluator()
        registry.register(evaluator)
        
        found = registry.get_by_name("EconomicFunctionEvaluator")
        
        assert found is evaluator
    
    def test_get_by_name_not_found(self, registry):
        """Test getting non-existent evaluator."""
        result = registry.get_by_name("NonExistentEvaluator")
        
        assert result is None
    
    @pytest.mark.asyncio
    async def test_evaluate_all(self, registry):
        """Test evaluating with all registered evaluators."""
        registry.register(EconomicFunctionEvaluator())
        registry.register(StoryOperationEvaluator())
        
        intent = "Adjust revenue forecast"
        context = {}
        
        results = await registry.evaluate_all(intent, context)
        
        assert len(results) == 2
        # Economic evaluator should handle this
        assert any(r.can_handle and "Economic" in r.evaluator_name for r in results)
    
    def test_get_capabilities(self, registry):
        """Test getting capabilities of all evaluators."""
        registry.register(EconomicFunctionEvaluator())
        registry.register(StoryOperationEvaluator())
        
        caps = registry.get_capabilities()
        
        assert caps["total_count"] == 2
        assert len(caps["evaluators"]) == 2


class TestDefaultRegistry:
    """Test default registry creation."""
    
    def test_create_default_registry(self):
        """Test creating default registry with all evaluators."""
        registry = create_default_registry()
        
        # Should have all 5 default evaluators
        assert len(registry.get_all()) == 5
        
        # Verify all evaluators present
        assert registry.get_by_name("EconomicFunctionEvaluator") is not None
        assert registry.get_by_name("StoryOperationEvaluator") is not None
        assert registry.get_by_name("ComparisonOperationEvaluator") is not None
        assert registry.get_by_name("ConstraintOperationEvaluator") is not None
        assert registry.get_by_name("CustomizationEvaluator") is not None
    
    @pytest.mark.asyncio
    async def test_default_registry_economic_intent(self):
        """Test default registry with economic intent."""
        registry = create_default_registry()
        
        intent = "Correlate revenue with market size"
        context = {}
        
        results = await registry.evaluate_all(intent, context)
        
        # Economic evaluator should handle this
        economic_results = [r for r in results if "Economic" in r.evaluator_name]
        assert len(economic_results) == 1
        assert economic_results[0].can_handle is True
    
    @pytest.mark.asyncio
    async def test_default_registry_story_intent(self):
        """Test default registry with story intent."""
        registry = create_default_registry()
        
        intent = "Create slide deck for investor presentation"
        context = {}
        
        results = await registry.evaluate_all(intent, context)
        
        # Story evaluator should handle this
        story_results = [r for r in results if "Story" in r.evaluator_name]
        assert len(story_results) == 1
        assert story_results[0].can_handle is True


class TestEvaluationResult:
    """Test EvaluationResult dataclass."""
    
    def test_evaluation_result_creation(self):
        """Test creating EvaluationResult."""
        result = EvaluationResult(
            can_handle=True,
            confidence=0.92,
            reasoning="Test reasoning",
            capability_metadata={"op": "test"},
            semantic_features={"feature1": True},
            evaluator_name="TestEvaluator",
        )
        
        assert result.can_handle is True
        assert result.confidence == 0.92
        assert result.reasoning == "Test reasoning"
        assert result.capability_metadata == {"op": "test"}
        assert result.semantic_features == {"feature1": True}
        assert result.evaluator_name == "TestEvaluator"
