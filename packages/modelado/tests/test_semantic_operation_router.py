"""Tests for Phase 9.2: Semantic Operation Router Integration.

Tests validate that:
1. SemanticOperationRouter correctly uses SemanticEngine for routing
2. Confidence thresholds work correctly
3. Fallback routing activates when needed
4. Handler selection maps correctly from evaluators
5. Integration with basic handlers works end-to-end
"""

import pytest
from datetime import datetime
from unittest.mock import AsyncMock, MagicMock, patch, Mock

from modelado.core.semantic_operation_router import (
    SemanticOperationRouter,
    SemanticGenerativeCommandRouter,
    SemanticRoutingDecision,
)
from modelado.core.generative_contracts import GenerativeCommand
from modelado.core.operation_handlers import (
    EconomicOperationHandler,
    StoryOperationHandler,
    SystemOperationHandler,
)
from modelado.semantic_engine import SemanticEvaluationResult
from modelado.intent_classifier import IntentClass


# Fixture for mock semantic engine
@pytest.fixture
def mock_semantic_engine():
    """Create a mock semantic engine."""
    engine = MagicMock()
    engine.evaluate = AsyncMock()
    return engine


class TestSemanticOperationRouter:
    """Test SemanticOperationRouter basic functionality."""
    
    @pytest.mark.asyncio
    async def test_route_with_high_confidence_evaluator(self, mock_semantic_engine):
        """Test routing when semantic evaluation has high confidence."""
        router = SemanticOperationRouter(semantic_engine=mock_semantic_engine, min_confidence=0.6)
        
        # Mock SemanticEngine result
        eval_result = SemanticEvaluationResult(
            intent="Adjust revenue by 15 percent",
            intent_class=IntentClass.ECONOMIC_FUNCTION,
            intent_confidence=0.95,
            intent_features=["revenue", "adjustment"],
            evaluator_name="EconomicFunctionEvaluator",
            evaluator_confidence=0.92,
            can_handle=True,
            reasoning="Detected revenue adjustment operation",
            all_evaluations=[],
            semantic_features={"revenue_detected": True, "adjustment_detected": True},
            capability_metadata={"operation_type": "revenue_adjustment"},
            used_generation=False,
            generation_reason=None,
        )
        
        # Configure the mock
        mock_semantic_engine.evaluate = AsyncMock(return_value=eval_result)
        
        # Create command
        command = GenerativeCommand.create(
            user_instruction="Adjust revenue by 15 percent",
            operation_type="economic_function",
            context={},
        )
        
        # Route
        decision = await router.route(command)
        
        # Validate
        assert decision.is_semantic_routed is True
        assert decision.selected_handler_name == "economic"
        assert decision.confidence == 0.92
        assert router.semantic_routed == 1
        assert router.fallback_routed == 0
    
    @pytest.mark.asyncio
    async def test_route_with_low_confidence_uses_fallback(self, mock_semantic_engine):
        """Test that low confidence triggers fallback routing."""
        router = SemanticOperationRouter(semantic_engine=mock_semantic_engine, min_confidence=0.8, use_fallback=True)
        
        # Mock low-confidence result
        eval_result = SemanticEvaluationResult(
            intent="Something economic-ish",
            intent_class=IntentClass.ECONOMIC_FUNCTION,
            intent_confidence=0.55,
            intent_features=["unclear"],
            evaluator_name="EconomicFunctionEvaluator",
            evaluator_confidence=0.45,  # Below threshold
            can_handle=True,
            reasoning="Low confidence match",
            all_evaluations=[],
            semantic_features={},
            capability_metadata={},
            used_generation=False,
            generation_reason=None,
        )
        
        mock_semantic_engine.evaluate = AsyncMock(return_value=eval_result)
        
        command = GenerativeCommand.create(
            user_instruction="Something economic-ish",
            operation_type="economic_function",
            context={},
        )
        
        decision = await router.route(command)
        
        # Should use fallback
        assert decision.is_semantic_routed is False
        assert decision.selected_handler_name == "economic"  # Fallback via operation_type
        assert router.confidence_rejections == 1
        assert router.fallback_routed == 1
    
    @pytest.mark.asyncio
    async def test_route_no_evaluator_found_uses_fallback(self, mock_semantic_engine):
        """Test that when no evaluator matches, fallback routing activates."""
        router = SemanticOperationRouter(semantic_engine=mock_semantic_engine, use_fallback=True)
        
        # Mock: no evaluator can handle
        eval_result = SemanticEvaluationResult(
            intent="Something novel and strange",
            intent_class=IntentClass.UNKNOWN,
            intent_confidence=0.30,
            intent_features=[],
            evaluator_name=None,  # No evaluator found
            evaluator_confidence=0.0,
            can_handle=False,
            reasoning="No evaluator matched",
            all_evaluations=[],
            semantic_features={},
            capability_metadata={},
            used_generation=True,
            generation_reason="No evaluator matched; will generate novel operation",
        )
        
        mock_semantic_engine.evaluate = AsyncMock(return_value=eval_result)
        
        command = GenerativeCommand.create(
            user_instruction="Something novel and strange",
            operation_type="story_operation",
            context={},
        )
        
        decision = await router.route(command)
        
        # Should use fallback
        assert decision.is_semantic_routed is False
        assert decision.selected_handler_name == "story"  # Via operation_type
        assert router.fallback_routed == 1
    
    @pytest.mark.asyncio
    async def test_route_no_fallback_raises_error(self, mock_semantic_engine):
        """Test that when fallback disabled and semantic fails, error is raised."""
        router = SemanticOperationRouter(semantic_engine=mock_semantic_engine, use_fallback=False)
        
        eval_result = SemanticEvaluationResult(
            intent="Unknown operation",
            intent_class=IntentClass.UNKNOWN,
            intent_confidence=0.0,
            intent_features=[],
            evaluator_name=None,
            evaluator_confidence=0.0,
            can_handle=False,
            reasoning="Nothing matched",
            all_evaluations=[],
            semantic_features={},
            capability_metadata={},
            used_generation=True,
            generation_reason="No evaluator matched; will generate novel operation",
        )
        
        mock_semantic_engine.evaluate = AsyncMock(return_value=eval_result)
        
        command = GenerativeCommand.create(
            user_instruction="Unknown operation",
            operation_type="unknown_type",
            context={},
        )
        
        # Should raise error
        with pytest.raises(ValueError, match="Semantic routing failed"):
            await router.route(command)
        
        assert router.evaluation_errors == 1  # Error counted during routing failure
    
    def test_evaluator_to_handler_mapping(self, mock_semantic_engine):
        """Test that evaluators map to correct handlers."""
        router = SemanticOperationRouter(semantic_engine=mock_semantic_engine)
        
        assert router._map_evaluator_to_handler("EconomicFunctionEvaluator") == "economic"
        assert router._map_evaluator_to_handler("StoryOperationEvaluator") == "story"
        assert router._map_evaluator_to_handler("SystemOperationEvaluator") == "system"
        assert router._map_evaluator_to_handler("UnknownEvaluator") == "default"
    
    def test_operation_type_to_handler_mapping(self, mock_semantic_engine):
        """Test fallback mapping from operation_type to handler."""
        router = SemanticOperationRouter(semantic_engine=mock_semantic_engine)
        
        assert router._map_operation_type_to_handler("economic_function") == "economic"
        assert router._map_operation_type_to_handler("story_operation") == "story"
        assert router._map_operation_type_to_handler("system_operation") == "system"
        assert router._map_operation_type_to_handler("unknown") == "system"


class TestSemanticGenerativeCommandRouter:
    """Test integrated semantic + generative command router."""
    
    @pytest.mark.asyncio
    async def test_route_semantic_with_economic_handler(self, mock_semantic_engine):
        """Test end-to-end semantic routing with actual handler."""
        # Create router with handlers
        semantic_router = SemanticOperationRouter(semantic_engine=mock_semantic_engine)
        router = SemanticGenerativeCommandRouter(semantic_router=semantic_router)
        router.register_handler(EconomicOperationHandler())
        
        # Mock semantic evaluation
        eval_result = SemanticEvaluationResult(
            intent="Adjust revenue by 15 percent",
            intent_class=IntentClass.ECONOMIC_FUNCTION,
            intent_confidence=0.95,
            intent_features=["revenue"],
            evaluator_name="EconomicFunctionEvaluator",
            evaluator_confidence=0.90,
            can_handle=True,
            reasoning="Economic revenue adjustment",
            all_evaluations=[],
            semantic_features={"revenue_detected": True},
            capability_metadata={},
            used_generation=False,
            generation_reason=None,
        )
        
        mock_semantic_engine.evaluate = AsyncMock(return_value=eval_result)
        
        # Create and route command
        command = GenerativeCommand.create(
            user_instruction="Adjust revenue by 15 percent",
            operation_type="economic_function",
            context={},
        )
        
        decision, operation = await router.route_semantic(command)
        
        # Validate routing decision
        assert decision.is_semantic_routed is True
        assert decision.selected_handler_name == "economic"
        
        # Validate generated operation
        assert operation is not None
        expected_intent = operation.generation_metadata.get("intent_type")
        assert expected_intent in operation.generated_function.name
        assert "revenue" in operation.generated_function.code.lower()
    
    @pytest.mark.asyncio
    async def test_route_semantic_with_story_handler(self, mock_semantic_engine):
        """Test semantic routing with story handler."""
        semantic_router = SemanticOperationRouter(semantic_engine=mock_semantic_engine)
        router = SemanticGenerativeCommandRouter(semantic_router=semantic_router)
        router.register_handler(StoryOperationHandler())
        
        eval_result = SemanticEvaluationResult(
            intent="Create investor pitch narrative",
            intent_class=IntentClass.STORY_OPERATION,
            intent_confidence=0.92,
            intent_features=["narrative", "investor"],
            evaluator_name="StoryOperationEvaluator",
            evaluator_confidence=0.88,
            can_handle=True,
            reasoning="Story narrative generation",
            all_evaluations=[],
            semantic_features={"narrative_detected": True},
            capability_metadata={},
            used_generation=False,
            generation_reason=None,
        )
        
        mock_semantic_engine.evaluate = AsyncMock(return_value=eval_result)
        
        command = GenerativeCommand.create(
            user_instruction="Create investor pitch narrative",
            operation_type="story_operation",
            context={},
        )
        
        decision, operation = await router.route_semantic(command)
        
        # Validate
        assert decision.selected_handler_name == "story"
        expected_intent = operation.generation_metadata.get("intent_type")
        assert expected_intent in operation.generated_function.name
        assert "narrative" in operation.generated_function.code.lower()
    
    def test_semantic_stats(self, mock_semantic_engine):
        """Test that semantic stats are collected."""
        semantic_router = SemanticOperationRouter(semantic_engine=mock_semantic_engine)
        router = SemanticGenerativeCommandRouter(semantic_router=semantic_router)
        
        stats = router.get_semantic_stats()
        
        assert 'semantic_router' in stats
        assert 'generative_router' in stats
        assert stats['semantic_router']['total_commands'] == 0
        assert stats['semantic_router']['semantic_routed'] == 0


class TestPhase92Integration:
    """Integration tests for Phase 9.2 overall."""
    
    @pytest.mark.asyncio
    async def test_semantic_routing_with_all_handlers(self, mock_semantic_engine):
        """Test that all three handler types work with semantic routing."""
        semantic_router = SemanticOperationRouter(semantic_engine=mock_semantic_engine)
        router = SemanticGenerativeCommandRouter(semantic_router=semantic_router)
        router.register_handler(EconomicOperationHandler())
        router.register_handler(StoryOperationHandler())
        router.register_handler(SystemOperationHandler())
        
        test_cases = [
            ("Forecast revenue growth for next quarter", "economic", IntentClass.ECONOMIC_FUNCTION, "economic_function", "EconomicFunctionEvaluator"),
            ("Create narrative arc for series A story", "story", IntentClass.STORY_OPERATION, "story_operation", "StoryOperationEvaluator"),
            ("Generate system health report", "system", IntentClass.SYSTEM_OPERATION, "system_operation", "SystemOperationEvaluator"),
        ]
        
        for intent, expected_handler, expected_class, op_type, evaluator_name in test_cases:
            eval_result = SemanticEvaluationResult(
                intent=intent,
                intent_class=expected_class,
                intent_confidence=0.90,
                intent_features=["test"],
                evaluator_name=evaluator_name,
                evaluator_confidence=0.88,
                can_handle=True,
                reasoning="Test evaluation",
                all_evaluations=[],
                semantic_features={},
                capability_metadata={},
                used_generation=False,
                generation_reason=None,
            )
            
            mock_semantic_engine.evaluate = AsyncMock(return_value=eval_result)
            
            command = GenerativeCommand.create(
                user_instruction=intent,
                operation_type=op_type,
                context={},
            )
            
            decision, operation = await router.route_semantic(command)
            
            # Validate
            assert decision.is_semantic_routed is True
            assert decision.selected_handler_name == expected_handler
            assert operation.generated_function is not None
            assert operation.can_execute() is True
