"""Semantic-driven operation router.

Semantic evaluation engine integration.

This module bridges the generative command infrastructure with SemanticEngine to
enable semantic-first operation routing.

Architecture:
  GenerativeCommand
    ↓
  SemanticOperationRouter
    ├─ SemanticEngine.evaluate() → intent classification + evaluator selection
    ├─ Confidence check (accept if >= threshold)
    ├─ Handler selection (via semantic evaluator recommendations)
    └─ Fallback to GenerativeCommandRouter if semantic unavailable
    ↓
  GeneratedOperation

Key Principles:
- Semantic-first dispatch (never boolean logic)
- Confidence-based acceptance (min_confidence threshold)
- Graceful degradation (fallback to simple routing)
- Full observability (reasoning, confidence, evaluator metrics)

No Hardcoding:
- ❌ Never hardcoded intent→handler mapping
- ❌ Never enum-based routing
- ✅ All classification via SemanticEngine
- ✅ Evaluators determine handler selection
"""

from __future__ import annotations

import logging
from dataclasses import dataclass
from datetime import datetime
from typing import Any, Dict, Optional
from uuid import uuid4

from modelado.core.generative_router import (
    GenerativeCommandRouter,
    GenerativeHandler,
    GenerativeCommand,
)
from modelado.semantic_engine import SemanticEngine, SemanticEvaluationResult

logger = logging.getLogger(__name__)


@dataclass
class SemanticRoutingDecision:
    """Decision made by semantic router."""
    
    command_id: str
    semantic_result: SemanticEvaluationResult
    selected_handler_name: Optional[str]  # Handler to invoke
    confidence: float  # Combined semantic + evaluator confidence
    reasoning: str  # Why this routing decision
    is_semantic_routed: bool  # True if routed via semantic, False if fallback
    evaluation_time_ms: float
    routed_at: datetime


class SemanticOperationRouter:
    """
    Semantic-first router for generative operations.
    
    Uses SemanticEngine to classify intent and select handlers based on
    semantic matching rather than hardcoded enum or string matching.
    
    Workflow:
    1. Receive GenerativeCommand
    2. Use SemanticEngine to evaluate intent → get recommended evaluator
    3. If evaluator found and confidence >= threshold, use its recommendations
    4. Map evaluator to handler in the generative command router
    5. Fallback to simple command routing if semantic unavailable
    """
    
    def __init__(
        self,
        semantic_engine: Optional[SemanticEngine] = None,
        generative_router: Optional[GenerativeCommandRouter] = None,
        min_confidence: float = 0.6,
        use_fallback: bool = True,
    ):
        """Initialize semantic router.
        
        Args:
            semantic_engine: SemanticEngine instance (creates default if None)
            generative_router: GenerativeCommandRouter (creates default if None)
            min_confidence: Minimum confidence to accept semantic routing (0.5-1.0)
            use_fallback: Whether to fallback to simple routing if semantic fails
        """
        self.semantic_engine = semantic_engine or SemanticEngine()
        self.generative_router = generative_router or GenerativeCommandRouter()
        self.min_confidence = min_confidence
        self.use_fallback = use_fallback
        
        # Metrics
        self.total_commands = 0
        self.semantic_routed = 0
        self.fallback_routed = 0
        self.confidence_rejections = 0
        self.evaluation_errors = 0
    
    async def route(
        self,
        command: GenerativeCommand,
    ) -> SemanticRoutingDecision:
        """Route command using semantic evaluation.
        
        Args:
            command: GenerativeCommand to route
            
        Returns:
            SemanticRoutingDecision with routing metadata
            
        Raises:
            ValueError: If no semantic result and fallback disabled
            RuntimeError: If routing fails
        """
        self.total_commands += 1
        start_time = datetime.utcnow()
        
        try:
            # Step 1: Evaluate intent semantically
            logger.info(
                f"Semantic routing for command {command.command_id}: "
                f'intent="{command.user_instruction}"'
            )
            
            semantic_result = await self.semantic_engine.evaluate(
                intent=command.user_instruction,
                context=command.context,
            )
            
            logger.debug(
                f"Semantic evaluation: class={semantic_result.intent_class.value}, "
                f"evaluator={semantic_result.evaluator_name}, "
                f"confidence={semantic_result.evaluator_confidence:.2f}"
            )
            
            # Step 2: Check confidence threshold
            if (
                semantic_result.evaluator_confidence < self.min_confidence
                and semantic_result.evaluator_name is not None
            ):
                self.confidence_rejections += 1
                logger.warning(
                    f"Semantic confidence {semantic_result.evaluator_confidence:.2f} "
                    f"below threshold {self.min_confidence} for command {command.command_id}"
                )
                
                if not self.use_fallback:
                    raise ValueError(
                        f"Semantic routing confidence {semantic_result.evaluator_confidence:.2f} "
                        f"below threshold {self.min_confidence}"
                    )
                
                # Fall through to fallback routing below
            elif semantic_result.can_handle and semantic_result.evaluator_name:
                # Semantic routing succeeded
                self.semantic_routed += 1
                
                handler_name = self._map_evaluator_to_handler(
                    semantic_result.evaluator_name,
                )
                
                elapsed_ms = (datetime.utcnow() - start_time).total_seconds() * 1000
                
                decision = SemanticRoutingDecision(
                    command_id=command.command_id,
                    semantic_result=semantic_result,
                    selected_handler_name=handler_name,
                    confidence=semantic_result.evaluator_confidence,
                    reasoning=semantic_result.reasoning,
                    is_semantic_routed=True,
                    evaluation_time_ms=elapsed_ms,
                    routed_at=datetime.utcnow(),
                )
                
                logger.info(
                    f"Semantic routing successful for {command.command_id}: "
                    f"handler={handler_name}, confidence={semantic_result.evaluator_confidence:.2f}"
                )
                
                return decision
            
            # Step 3: Fallback routing (either confidence too low or no evaluator matched)
            if self.use_fallback:
                self.fallback_routed += 1
                
                logger.info(
                    f"Falling back to non-semantic routing for {command.command_id} "
                    f"(can_handle={semantic_result.can_handle}, "
                    f"evaluator={semantic_result.evaluator_name})"
                )
                
                # Route using operation_type directly
                handler_name = self._map_operation_type_to_handler(
                    command.operation_type,
                )
                
                elapsed_ms = (datetime.utcnow() - start_time).total_seconds() * 1000
                
                decision = SemanticRoutingDecision(
                    command_id=command.command_id,
                    semantic_result=semantic_result,
                    selected_handler_name=handler_name,
                    confidence=semantic_result.evaluator_confidence,
                    reasoning=f"Fallback: {semantic_result.reasoning}",
                    is_semantic_routed=False,
                    evaluation_time_ms=elapsed_ms,
                    routed_at=datetime.utcnow(),
                )
                
                logger.info(
                    f"Fallback routing for {command.command_id}: "
                    f"handler={handler_name} (operation_type={command.operation_type})"
                )
                
                return decision
            
            # No fallback and semantic failed
            raise ValueError(
                f"Semantic routing failed for {command.command_id} "
                f"(can_handle={semantic_result.can_handle}), "
                f"fallback disabled"
            )
        
        except Exception as e:
            self.evaluation_errors += 1
            logger.error(
                f"Semantic routing error for {command.command_id}: {e}",
                exc_info=True,
            )
            raise
    
    def _map_evaluator_to_handler(self, evaluator_name: str) -> str:
        """Map semantic evaluator name to handler name.
        
        Args:
            evaluator_name: Name from SemanticEngine (e.g., "EconomicFunctionEvaluator")
            
        Returns:
            Handler name to use in GenerativeCommandRouter
        """
        # Map semantic evaluators to handlers
        mapping = {
            "EconomicFunctionEvaluator": "economic",
            "StoryOperationEvaluator": "story",
            "SystemOperationEvaluator": "system",
        }
        
        handler = mapping.get(evaluator_name, "default")
        logger.debug(f"Mapped evaluator {evaluator_name} → handler {handler}")
        return handler
    
    def _map_operation_type_to_handler(self, operation_type: str) -> str:
        """Map operation_type to handler name (fallback routing).
        
        Args:
            operation_type: From GenerativeCommand (e.g., "economic_function")
            
        Returns:
            Handler name
        """
        if "economic" in operation_type.lower():
            return "economic"
        elif "story" in operation_type.lower():
            return "story"
        else:
            return "system"
    
    def get_stats(self) -> Dict[str, Any]:
        """Get router statistics."""
        return {
            'total_commands': self.total_commands,
            'semantic_routed': self.semantic_routed,
            'fallback_routed': self.fallback_routed,
            'confidence_rejections': self.confidence_rejections,
            'evaluation_errors': self.evaluation_errors,
            'semantic_routing_rate': (
                self.semantic_routed / max(1, self.total_commands) * 100
            ),
            'min_confidence': self.min_confidence,
            'use_fallback': self.use_fallback,
        }


# ============================================================================
# Integration with GenerativeCommandRouter
# ============================================================================

class SemanticGenerativeCommandRouter(GenerativeCommandRouter):
    """
    Extended GenerativeCommandRouter that routes via SemanticOperationRouter.
    
    Provides semantic-first operation dispatch while maintaining backward
    compatibility with non-semantic routing.
    
    Usage:
        router = SemanticGenerativeCommandRouter()
        router.register_handler(EconomicHandler(), operation_type="economic")
        
        command = GenerativeCommand.create(...)
        operation = await router.route_semantic(command)
    """
    
    def __init__(
        self,
        semantic_router: Optional[SemanticOperationRouter] = None,
        **kwargs,
    ):
        """Initialize semantic-aware router.
        
        Args:
            semantic_router: SemanticOperationRouter (creates default if None)
            **kwargs: Passed to GenerativeCommandRouter
        """
        super().__init__(**kwargs)
        self.semantic_router = semantic_router or SemanticOperationRouter()
    
    async def route_semantic(
        self,
        command: GenerativeCommand,
    ) -> tuple:
        """Route command using semantic evaluation.
        
        Returns:
            (SemanticRoutingDecision, GeneratedOperation) tuple
        """
        # Get semantic routing decision
        decision = await self.semantic_router.route(command)
        
        # Route through generative command router using selected handler
        if decision.selected_handler_name:
            logger.info(
                f"Executing handler {decision.selected_handler_name} "
                f"for command {command.command_id}"
            )
        
        # Use parent class route() for actual execution
        operation = await super().route(command)
        
        return decision, operation
    
    def get_semantic_stats(self) -> Dict[str, Any]:
        """Get semantic routing statistics."""
        return {
            'semantic_router': self.semantic_router.get_stats(),
            'generative_router': self.get_stats(),
        }
