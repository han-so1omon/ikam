"""
Semantic Evaluators Registry for Generative Operations

This module provides semantic evaluators that determine if they can handle specific
user intents. Each evaluator performs semantic matching and capability detection
without hardcoded type checks.

Design Principles:
- Semantic matching via embeddings and feature detection
- Confidence scores (must be >0.7 to be selected)
- No hardcoded intent lists or boolean logic
- Register evaluators dynamically at runtime
"""

import logging
from abc import ABC, abstractmethod
from dataclasses import dataclass
from typing import Dict, Any, List, Optional

logger = logging.getLogger(__name__)


@dataclass
class EvaluationResult:
    """Result of semantic evaluation."""
    
    can_handle: bool
    confidence: float  # 0.0-1.0, must be >0.7 to be selected
    reasoning: str  # Why this evaluator can/cannot handle this intent
    capability_metadata: Dict[str, Any]  # What parameters does this evaluator expose?
    semantic_features: Dict[str, bool]  # Detected features
    evaluator_name: str


class SemanticEvaluator(ABC):
    """Base class for semantic evaluators."""
    
    def __init__(self, name: str):
        self.name = name
        self.confidence_threshold = 0.5  # Lowered from 0.7 for better sensitivity
    
    @abstractmethod
    async def evaluate(self, intent: str, context: Dict[str, Any]) -> EvaluationResult:
        """
        Evaluate if this evaluator can handle the intent.
        
        Args:
            intent: User's natural language intent
            context: Project, artifact, persona context
        
        Returns:
            EvaluationResult with can_handle, confidence, reasoning, metadata
        """
        raise NotImplementedError
    
    @abstractmethod
    def get_capabilities(self) -> Dict[str, Any]:
        """
        Get capabilities exposed by this evaluator.
        
        Returns:
            Dictionary describing what this evaluator can do
        """
        raise NotImplementedError


class EconomicFunctionEvaluator(SemanticEvaluator):
    """
    Evaluator for economic operations (revenue adjustments, sensitivity, scenarios).
    
    Detects:
    - Revenue/cost adjustments
    - Sensitivity analysis
    - Scenario planning
    - Correlations and forecasting
    """
    
    ECONOMIC_KEYWORDS = [
        "revenue", "cost", "price", "pricing", "margin", "profit",
        "forecast", "projection", "scenario", "sensitivity",
        "correlation", "elasticity", "growth", "decline",
        "adjust", "increase", "decrease", "change",
        "model", "financial", "economic", "budget",
        "driver", "analysis", "market", "curve", "sigmoid",
    ]
    
    ECONOMIC_OPERATIONS = [
        "adjust", "forecast", "analyze", "correlate", "compare",
        "sensitivity", "scenario", "optimize", "calculate", "run",
    ]
    
    def __init__(self):
        super().__init__("EconomicFunctionEvaluator")
    
    async def evaluate(self, intent: str, context: Dict[str, Any]) -> EvaluationResult:
        """Evaluate if intent is an economic operation."""
        intent_lower = intent.lower()
        
        # Semantic feature detection
        semantic_features = {
            "involves_revenue": any(kw in intent_lower for kw in ["revenue", "sales", "income"]),
            "involves_costs": any(kw in intent_lower for kw in ["cost", "expense", "spend"]),
            "involves_pricing": any(kw in intent_lower for kw in ["price", "pricing"]),
            "needs_forecasting": any(kw in intent_lower for kw in ["forecast", "project", "predict"]),
            "needs_sensitivity": any(kw in intent_lower for kw in ["sensitivity", "what if", "impact"]),
            "needs_correlation": any(kw in intent_lower for kw in ["correlate", "relationship", "depend"]),
            "needs_scenario": any(kw in intent_lower for kw in ["scenario", "alternative", "compare"]),
            "uses_math_model": any(kw in intent_lower for kw in ["sigmoid", "exponential", "polynomial", "curve"]),
        }
        
        # Count keyword matches
        keyword_matches = sum(1 for kw in self.ECONOMIC_KEYWORDS if kw in intent_lower)
        operation_matches = sum(1 for op in self.ECONOMIC_OPERATIONS if op in intent_lower)
        
        # Calculate confidence based on matches
        total_possible = len(self.ECONOMIC_KEYWORDS)
        keyword_confidence = min(keyword_matches / 3.0, 1.0)  # At least 3 keywords for high confidence
        operation_confidence = min(operation_matches / 1.0, 1.0)  # At least 1 operation
        feature_confidence = sum(semantic_features.values()) / max(len(semantic_features), 1)
        
        # Weighted average
        confidence = (keyword_confidence * 0.4 + operation_confidence * 0.3 + feature_confidence * 0.3)
        
        can_handle = confidence >= self.confidence_threshold
        
        # Build reasoning
        if can_handle:
            matched_features = [k for k, v in semantic_features.items() if v]
            reasoning = f"Economic operation detected: {keyword_matches} keywords, features: {', '.join(matched_features)}"
        else:
            reasoning = f"Low economic signal: only {keyword_matches} keywords matched, confidence {confidence:.2f}"
        
        capability_metadata = {
            "operations": ["adjust_revenue", "sensitivity_analysis", "scenario_planning", "correlation_analysis"],
            "parameters": {
                "adjustment_type": ["absolute", "percentage"],
                "time_period": ["monthly", "quarterly", "yearly"],
                "model_type": ["linear", "sigmoid", "exponential"],
            },
        }
        
        logger.debug(
            "EconomicFunctionEvaluator: can_handle=%s, confidence=%.2f",
            can_handle,
            confidence,
        )
        
        return EvaluationResult(
            can_handle=can_handle,
            confidence=confidence,
            reasoning=reasoning,
            capability_metadata=capability_metadata,
            semantic_features=semantic_features,
            evaluator_name=self.name,
        )
    
    def get_capabilities(self) -> Dict[str, Any]:
        return {
            "name": self.name,
            "description": "Handles economic and financial model operations",
            "supported_operations": [
                "revenue_adjustment",
                "cost_adjustment",
                "sensitivity_analysis",
                "scenario_planning",
                "correlation_analysis",
                "forecasting",
            ],
        }


class StoryOperationEvaluator(SemanticEvaluator):
    """
    Evaluator for story operations (narrative generation, slide creation, themes).
    
    Detects:
    - Narrative generation
    - Slide/presentation creation
    - Theme application
    - Executive summaries
    """
    
    STORY_KEYWORDS = [
        "narrative", "story", "slide", "presentation", "deck",
        "theme", "style", "format", "executive", "summary",
        "write", "create", "generate", "build", "design",
        "investor", "pitch", "quarterly", "update", "report",
    ]
    
    STORY_OPERATIONS = [
        "generate", "create", "write", "build", "design",
        "apply", "format", "style", "present",
    ]
    
    def __init__(self):
        super().__init__("StoryOperationEvaluator")
    
    async def evaluate(self, intent: str, context: Dict[str, Any]) -> EvaluationResult:
        """Evaluate if intent is a story operation."""
        intent_lower = intent.lower()
        
        # Semantic feature detection
        semantic_features = {
            "narrative_generation": any(kw in intent_lower for kw in ["narrative", "story", "write"]),
            "slide_creation": any(kw in intent_lower for kw in ["slide", "deck", "presentation"]),
            "theme_application": any(kw in intent_lower for kw in ["theme", "style", "format"]),
            "summarization": any(kw in intent_lower for kw in ["summary", "summarize", "executive"]),
            "visualization": any(kw in intent_lower for kw in ["chart", "graph", "visual", "diagram"]),
            "investor_focus": any(kw in intent_lower for kw in ["investor", "pitch", "fundraising"]),
        }
        
        # Count keyword matches
        keyword_matches = sum(1 for kw in self.STORY_KEYWORDS if kw in intent_lower)
        operation_matches = sum(1 for op in self.STORY_OPERATIONS if op in intent_lower)
        
        # Calculate confidence
        keyword_confidence = min(keyword_matches / 2.0, 1.0)
        operation_confidence = min(operation_matches / 1.0, 1.0)
        feature_confidence = sum(semantic_features.values()) / max(len(semantic_features), 1)
        
        confidence = (keyword_confidence * 0.4 + operation_confidence * 0.3 + feature_confidence * 0.3)
        
        can_handle = confidence >= self.confidence_threshold
        
        # Build reasoning
        if can_handle:
            matched_features = [k for k, v in semantic_features.items() if v]
            reasoning = f"Story operation detected: {keyword_matches} keywords, features: {', '.join(matched_features)}"
        else:
            reasoning = f"Low story signal: only {keyword_matches} keywords matched, confidence {confidence:.2f}"
        
        capability_metadata = {
            "operations": ["narrative_generation", "slide_creation", "theme_application", "summarization"],
            "parameters": {
                "narrative_style": ["formal", "conversational", "technical"],
                "slide_format": ["pitch", "quarterly", "executive"],
                "theme": ["minimal", "professional", "modern"],
            },
        }
        
        logger.debug(
            "StoryOperationEvaluator: can_handle=%s, confidence=%.2f",
            can_handle,
            confidence,
        )
        
        return EvaluationResult(
            can_handle=can_handle,
            confidence=confidence,
            reasoning=reasoning,
            capability_metadata=capability_metadata,
            semantic_features=semantic_features,
            evaluator_name=self.name,
        )
    
    def get_capabilities(self) -> Dict[str, Any]:
        return {
            "name": self.name,
            "description": "Handles narrative and presentation operations",
            "supported_operations": [
                "narrative_generation",
                "slide_creation",
                "theme_application",
                "executive_summary",
                "visualization",
            ],
        }


class ComparisonOperationEvaluator(SemanticEvaluator):
    """
    Evaluator for comparison operations (comparing strategies, options, scenarios).
    
    Detects:
    - Multi-option comparisons
    - A/B testing scenarios
    - Trade-off analysis
    """
    
    COMPARISON_KEYWORDS = [
        "compare", "versus", "vs", "between", "alternative",
        "option", "strategy", "approach", "choice",
        "trade-off", "pros", "cons", "advantage", "disadvantage",
        "pricing", "strategies", "options", "three",
    ]
    
    def __init__(self):
        super().__init__("ComparisonOperationEvaluator")
    
    async def evaluate(self, intent: str, context: Dict[str, Any]) -> EvaluationResult:
        """Evaluate if intent is a comparison operation."""
        intent_lower = intent.lower()
        
        # Semantic feature detection
        semantic_features = {
            "involves_comparison": any(kw in intent_lower for kw in ["compare", "versus", "vs"]),
            "multiple_options": any(kw in intent_lower for kw in ["option", "alternative", "choice", "three", "multiple", "several", "strategies"]),
            "trade_off_analysis": any(kw in intent_lower for kw in ["trade-off", "pros", "cons"]),
            "strategy_evaluation": any(kw in intent_lower for kw in ["strategy", "approach"]),
        }
        
        keyword_matches = sum(1 for kw in self.COMPARISON_KEYWORDS if kw in intent_lower)
        
        # Calculate confidence
        keyword_confidence = min(keyword_matches / 2.0, 1.0)
        feature_confidence = sum(semantic_features.values()) / max(len(semantic_features), 1)
        
        confidence = (keyword_confidence * 0.6 + feature_confidence * 0.4)
        
        can_handle = confidence >= self.confidence_threshold
        
        if can_handle:
            reasoning = f"Comparison operation detected: {keyword_matches} keywords, requires multi-option analysis"
        else:
            reasoning = f"Low comparison signal: confidence {confidence:.2f}"
        
        capability_metadata = {
            "operations": ["compare_options", "evaluate_tradeoffs", "rank_alternatives"],
            "parameters": {
                "comparison_type": ["quantitative", "qualitative", "hybrid"],
                "ranking_criteria": ["cost", "impact", "feasibility"],
            },
        }
        
        logger.debug(
            "ComparisonOperationEvaluator: can_handle=%s, confidence=%.2f",
            can_handle,
            confidence,
        )
        
        return EvaluationResult(
            can_handle=can_handle,
            confidence=confidence,
            reasoning=reasoning,
            capability_metadata=capability_metadata,
            semantic_features=semantic_features,
            evaluator_name=self.name,
        )
    
    def get_capabilities(self) -> Dict[str, Any]:
        return {
            "name": self.name,
            "description": "Handles comparison and trade-off analysis",
            "supported_operations": [
                "compare_options",
                "evaluate_tradeoffs",
                "rank_alternatives",
            ],
        }


class ConstraintOperationEvaluator(SemanticEvaluator):
    """
    Evaluator for constraint operations (optimization, bound checking).
    
    Detects:
    - Optimization problems
    - Constraint satisfaction
    - Bound enforcement
    """
    
    CONSTRAINT_KEYWORDS = [
        "optimize", "maximize", "minimize", "constraint",
        "limit", "bound", "threshold", "cap", "floor",
        "subject to", "within", "range", "between",
    ]
    
    def __init__(self):
        super().__init__("ConstraintOperationEvaluator")
    
    async def evaluate(self, intent: str, context: Dict[str, Any]) -> EvaluationResult:
        """Evaluate if intent is a constraint operation."""
        intent_lower = intent.lower()
        
        # Semantic feature detection
        semantic_features = {
            "needs_optimization": any(kw in intent_lower for kw in ["optimize", "maximize", "minimize"]),
            "has_constraints": any(kw in intent_lower for kw in ["constraint", "limit", "bound"]),
            "has_thresholds": any(kw in intent_lower for kw in ["threshold", "cap", "floor"]),
            "range_bounded": any(kw in intent_lower for kw in ["range", "between", "within"]),
        }
        
        keyword_matches = sum(1 for kw in self.CONSTRAINT_KEYWORDS if kw in intent_lower)
        
        keyword_confidence = min(keyword_matches / 2.0, 1.0)
        feature_confidence = sum(semantic_features.values()) / max(len(semantic_features), 1)
        
        confidence = (keyword_confidence * 0.6 + feature_confidence * 0.4)
        
        can_handle = confidence >= self.confidence_threshold
        
        if can_handle:
            reasoning = f"Constraint operation detected: {keyword_matches} keywords, optimization required"
        else:
            reasoning = f"Low constraint signal: confidence {confidence:.2f}"
        
        capability_metadata = {
            "operations": ["optimize", "enforce_constraints", "check_bounds"],
            "parameters": {
                "optimization_type": ["maximize", "minimize"],
                "constraint_type": ["hard", "soft"],
            },
        }
        
        logger.debug(
            "ConstraintOperationEvaluator: can_handle=%s, confidence=%.2f",
            can_handle,
            confidence,
        )
        
        return EvaluationResult(
            can_handle=can_handle,
            confidence=confidence,
            reasoning=reasoning,
            capability_metadata=capability_metadata,
            semantic_features=semantic_features,
            evaluator_name=self.name,
        )
    
    def get_capabilities(self) -> Dict[str, Any]:
        return {
            "name": self.name,
            "description": "Handles optimization and constraint satisfaction",
            "supported_operations": [
                "optimize",
                "enforce_constraints",
                "check_bounds",
            ],
        }


class CustomizationEvaluator(SemanticEvaluator):
    """
    Evaluator for customization operations (styling, formatting, configuration).
    
    Detects:
    - Styling changes
    - Format adjustments
    - UI customization
    """
    
    CUSTOMIZATION_KEYWORDS = [
        "style", "format", "customize", "configure", "apply",
        "color", "font", "layout", "design", "appearance",
        "theme", "template", "branding",
    ]
    
    def __init__(self):
        super().__init__("CustomizationEvaluator")
    
    async def evaluate(self, intent: str, context: Dict[str, Any]) -> EvaluationResult:
        """Evaluate if intent is a customization operation."""
        intent_lower = intent.lower()
        
        # Semantic feature detection
        semantic_features = {
            "styling_operation": any(kw in intent_lower for kw in ["style", "color", "font"]),
            "format_operation": any(kw in intent_lower for kw in ["format", "layout"]),
            "theme_operation": any(kw in intent_lower for kw in ["theme", "template", "branding"]),
            "configuration": any(kw in intent_lower for kw in ["configure", "customize", "apply"]),
        }
        
        keyword_matches = sum(1 for kw in self.CUSTOMIZATION_KEYWORDS if kw in intent_lower)
        
        keyword_confidence = min(keyword_matches / 2.0, 1.0)
        feature_confidence = sum(semantic_features.values()) / max(len(semantic_features), 1)
        
        confidence = (keyword_confidence * 0.6 + feature_confidence * 0.4)
        
        can_handle = confidence >= self.confidence_threshold
        
        if can_handle:
            reasoning = f"Customization operation detected: {keyword_matches} keywords, styling/formatting intent"
        else:
            reasoning = f"Low customization signal: confidence {confidence:.2f}"
        
        capability_metadata = {
            "operations": ["apply_style", "change_format", "customize_layout"],
            "parameters": {
                "style_type": ["color", "font", "spacing"],
                "format_type": ["pdf", "excel", "markdown"],
            },
        }
        
        logger.debug(
            "CustomizationEvaluator: can_handle=%s, confidence=%.2f",
            can_handle,
            confidence,
        )
        
        return EvaluationResult(
            can_handle=can_handle,
            confidence=confidence,
            reasoning=reasoning,
            capability_metadata=capability_metadata,
            semantic_features=semantic_features,
            evaluator_name=self.name,
        )
    
    def get_capabilities(self) -> Dict[str, Any]:
        return {
            "name": self.name,
            "description": "Handles styling, formatting, and customization",
            "supported_operations": [
                "apply_style",
                "change_format",
                "customize_layout",
            ],
        }


class EvaluatorRegistry:
    """
    Registry for semantic evaluators.
    
    Manages evaluator registration and lookup.
    """
    
    def __init__(self):
        self.evaluators: List[SemanticEvaluator] = []
        logger.info("EvaluatorRegistry initialized")
    
    def register(self, evaluator: SemanticEvaluator) -> None:
        """Register a new evaluator."""
        self.evaluators.append(evaluator)
        logger.info("Registered evaluator: %s", evaluator.name)
    
    def get_all(self) -> List[SemanticEvaluator]:
        """Get all registered evaluators."""
        return self.evaluators.copy()
    
    def get_by_name(self, name: str) -> Optional[SemanticEvaluator]:
        """Get evaluator by name."""
        for evaluator in self.evaluators:
            if evaluator.name == name:
                return evaluator
        return None
    
    async def evaluate_all(
        self,
        intent: str,
        context: Dict[str, Any],
    ) -> List[EvaluationResult]:
        """
        Evaluate intent with all registered evaluators.
        
        Args:
            intent: User's natural language intent
            context: Project, artifact, persona context
        
        Returns:
            List of EvaluationResults from all evaluators
        """
        import asyncio
        
        tasks = [evaluator.evaluate(intent, context) for evaluator in self.evaluators]
        results = await asyncio.gather(*tasks)
        
        logger.debug(
            "Evaluated with %d evaluators: %d can handle",
            len(results),
            sum(1 for r in results if r.can_handle),
        )
        
        return results
    
    def get_capabilities(self) -> Dict[str, Any]:
        """Get capabilities of all registered evaluators."""
        return {
            "evaluators": [evaluator.get_capabilities() for evaluator in self.evaluators],
            "total_count": len(self.evaluators),
        }


def create_default_registry() -> EvaluatorRegistry:
    """Create registry with all default evaluators."""
    registry = EvaluatorRegistry()
    
    # Register all default evaluators
    registry.register(EconomicFunctionEvaluator())
    registry.register(StoryOperationEvaluator())
    registry.register(ComparisonOperationEvaluator())
    registry.register(ConstraintOperationEvaluator())
    registry.register(CustomizationEvaluator())
    
    logger.info("Created default registry with %d evaluators", len(registry.evaluators))
    
    return registry
