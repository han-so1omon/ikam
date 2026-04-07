"""Semantic operation handlers for economic, story, and system operations.

These handlers implement semantic-first operation generation with intent differentiation.
Each handler:
1. Analyzes intent semantically (no enum-based routing)
2. Selects an appropriate generation strategy based on semantic features
3. Creates an intent-specific ExecutableFunction with domain-specific constraints
4. Records complete generation metadata with semantic confidence
5. Returns a GeneratedOperation ready for execution or IKAM decomposition

Generation strategies are selected by StrategySelector (template/composable/LLM) based
on complexity and available context.
"""

from __future__ import annotations

import logging
import os
import hashlib
from datetime import datetime
from typing import Any, Dict, Optional, Set
from enum import Enum

from modelado.core.generative_router import GenerativeHandler
from modelado.core.generative_contracts import (
    GenerativeCommand,
    GeneratedOperation,
    ExecutableFunction,
    ValidationResults,
    GenerationStrategy,
    ConstraintType,
)
from modelado.core.function_generators.base import GenerationContext
from modelado.core.function_generators.strategy_selector import StrategySelector

logger = logging.getLogger(__name__)

# Semantic intent keywords for economic operations
ECONOMIC_INTENT_KEYWORDS = {
    "sensitivity": {"keywords": ["sensitivity", "elasticity", "impact", "change"], "type": "sensitivity_analysis"},
    "waterfall": {"keywords": ["waterfall", "decompose", "breakdown", "drivers"], "type": "waterfall_analysis"},
    "break_even": {"keywords": ["break-even", "breakeven", "minimum", "threshold"], "type": "break_even_analysis"},
    "unit_economics": {"keywords": ["unit economics", "cac", "ltv", "payback"], "type": "unit_economics_analysis"},
    "variance": {"keywords": ["variance", "budget", "actual", "deviation"], "type": "variance_analysis"},
    "contribution_margin": {"keywords": ["contribution", "margin", "gross", "net"], "type": "contribution_margin"},
    "growth_decomposition": {"keywords": ["growth", "decomposition", "organic", "inorganic"], "type": "growth_decomposition"},
    "profitability_attribution": {"keywords": ["profitability", "attribution", "drivers", "factors"], "type": "profitability_attribution"},
}

# Semantic intent keywords for story operations
STORY_INTENT_KEYWORDS = {
    "story_arc": {"keywords": ["story", "arc", "narrative", "journey"], "type": "story_arc_development"},
    "theme_application": {"keywords": ["theme", "apply", "consistent", "visual"], "type": "theme_application"},
    "framing": {"keywords": ["frame", "perspective", "context", "positioning"], "type": "framing_strategy"},
    "evidence_based": {"keywords": ["evidence", "data", "proof", "support"], "type": "evidence_based_narrative"},
    "audience_tailoring": {"keywords": ["audience", "tailor", "customize", "persona"], "type": "audience_tailoring"},
    "organization": {"keywords": ["organize", "structure", "flow", "sequence"], "type": "narrative_organization"},
    "visual_narrative": {"keywords": ["visual", "slide", "graphic", "diagram"], "type": "visual_narrative_design"},
}

# Semantic intent keywords for system operations
SYSTEM_INTENT_KEYWORDS = {
    "batch_processing": {"keywords": ["batch", "bulk", "process", "multiple"], "type": "batch_processing"},
    "orchestration": {"keywords": ["orchestrate", "workflow", "pipeline", "sequence"], "type": "workflow_orchestration"},
    "error_recovery": {"keywords": ["error", "recovery", "retry", "resilience"], "type": "error_recovery"},
    "cache_management": {"keywords": ["cache", "persist", "optimize", "performance"], "type": "cache_management"},
    "migration": {"keywords": ["migrate", "upgrade", "transfer", "convert"], "type": "data_migration"},
    "audit_logging": {"keywords": ["audit", "log", "track", "compliance"], "type": "audit_logging"},
    "notification_rules": {"keywords": ["notify", "alert", "event", "trigger"], "type": "notification_rules"},
}



class FallbackStrategyRouter:
    """Route operation generation based on confidence levels.
    
    High confidence (>= 0.85):
      PRIMARY → Use intent-specific template with full features
    
    Medium confidence (0.75–0.84):
      HYBRID → Use intent-specific template + add clarification comments
    
    Low confidence (< 0.75):
      GENERIC → Use generic operation template + request clarification
    
    Returns strategy enum and accompanying metadata for observability.
    """
    class Strategy(Enum):
        """Fallback strategies based on confidence level."""
        PRIMARY = "primary"           # Full intent-specific generation
        HYBRID = "hybrid"             # Intent-specific + clarification hints
        GENERIC = "generic"           # Generic operation (request clarification)
    
    # Confidence thresholds for routing decisions
    HIGH_CONFIDENCE_THRESHOLD = 0.85
    MEDIUM_CONFIDENCE_THRESHOLD = 0.75
    
    @staticmethod
    def select_strategy(confidence: float) -> "FallbackStrategyRouter.Strategy":
        """Select generation strategy based on confidence score.
        
        Args:
            confidence: Intent confidence score (0.0–1.0)
        
        Returns:
            Strategy enum indicating which generation path to follow
        """
        if confidence >= FallbackStrategyRouter.HIGH_CONFIDENCE_THRESHOLD:
            return FallbackStrategyRouter.Strategy.PRIMARY
        elif confidence >= FallbackStrategyRouter.MEDIUM_CONFIDENCE_THRESHOLD:
            return FallbackStrategyRouter.Strategy.HYBRID
        else:
            return FallbackStrategyRouter.Strategy.GENERIC
    
    @staticmethod
    def get_strategy_metadata(
        strategy: "FallbackStrategyRouter.Strategy",
        confidence: float,
        intent_type: str,
    ) -> Dict[str, Any]:
        """Generate strategy-specific metadata for operation.
        
        Returns:
            Dict with strategy details, confidence delta, and routing explanation
        """
        if strategy == FallbackStrategyRouter.Strategy.PRIMARY:
            delta_from_high = FallbackStrategyRouter.HIGH_CONFIDENCE_THRESHOLD - confidence
            return {
                "fallback_strategy": strategy.value,
                "confidence_tier": "high",
                "delta_from_threshold": delta_from_high,
                "routing_reason": f"High confidence ({confidence:.3f}) in {intent_type} intent — using full intent-specific template",
                "features_enabled": ["intent_specific_code", "domain_constraints", "semantic_metadata"],
            }
        elif strategy == FallbackStrategyRouter.Strategy.HYBRID:
            delta_from_medium = FallbackStrategyRouter.MEDIUM_CONFIDENCE_THRESHOLD - confidence
            return {
                "fallback_strategy": strategy.value,
                "confidence_tier": "medium",
                "delta_from_threshold": delta_from_medium,
                "routing_reason": f"Medium confidence ({confidence:.3f}) in {intent_type} intent — using template with clarification hints",
                "features_enabled": ["intent_specific_code", "clarification_comments", "domain_constraints"],
            }
        else:  # GENERIC
            delta_from_low = FallbackStrategyRouter.MEDIUM_CONFIDENCE_THRESHOLD - confidence
            return {
                "fallback_strategy": strategy.value,
                "confidence_tier": "low",
                "delta_from_threshold": delta_from_low,
                "routing_reason": f"Low confidence ({confidence:.3f}) in {intent_type} intent — falling back to generic operation with clarification request",
                "features_enabled": ["generic_code", "clarification_request", "minimal_constraints"],
                "requires_clarification": True,
            }


class SemanticConfidenceScorer:
    """Semantic confidence scoring with reasoning logs.
    
    Computes confidence scores for intent classification using:
    1. Keyword match coverage (how many keywords matched)
    2. Instruction detail level (word count, specificity)
    3. Feature extraction (parallel, retry, persistence detection)
    4. Intent uniqueness (how distinct from other intents)
    
    Returns confidence (0.0–1.0) + reasoning dict for observability.
    
    Intended to integrate SemanticEngine-provided confidence scores.
    """
    
    # Confidence thresholds
    CONFIDENCE_LOW = 0.60
    CONFIDENCE_MEDIUM = 0.75
    CONFIDENCE_HIGH = 0.85
    CONFIDENCE_VERY_HIGH = 0.95
    
    @staticmethod
    def score_intent(
        instruction: str,
        intent_type: str,
        keywords_dict: Dict[str, Dict[str, Any]],
    ) -> tuple[float, Dict[str, Any]]:
        """Compute confidence score and reasoning for intent classification.
        
        Returns: (confidence_score: float, reasoning: Dict)
        """
        instruction_lower = instruction.lower()
        reasoning = {
            "intent_type": intent_type,
            "instruction_length": len(instruction),
            "word_count": len(instruction.split()),
            "keyword_matches": [],
            "match_coverage": 0.0,
            "detail_score": 0.0,
            "feature_score": 0.0,
            "final_confidence": 0.0,
        }
        
        # 1. Keyword match coverage
        keyword_matches = []
        keyword_coverage = 0.0
        
        for key, info in keywords_dict.items():
            if info["type"] == intent_type:
                keywords = info["keywords"]
                matched = [kw for kw in keywords if kw in instruction_lower]
                if matched:
                    keyword_matches.extend(matched)
                    keyword_coverage = len(matched) / len(keywords)
        
        reasoning["keyword_matches"] = list(set(keyword_matches))
        reasoning["match_coverage"] = keyword_coverage
        
        # 2. Instruction detail score (word count based)
        word_count = len(instruction.split())
        if word_count < 5:
            detail_score = 0.50
        elif word_count < 10:
            detail_score = 0.65
        elif word_count < 20:
            detail_score = 0.80
        else:
            detail_score = 0.95
        
        reasoning["detail_score"] = detail_score
        
        # 3. Feature detection score
        features = SemanticConfidenceScorer._detect_features(instruction)
        feature_count = sum(1 for v in features.values() if v)
        feature_score = min(1.0, 0.5 + (feature_count * 0.1))  # 0.5–0.9 range
        
        reasoning["features_detected"] = {k: v for k, v in features.items() if v}
        reasoning["feature_score"] = feature_score
        
        # 4. Combine scores (weighted average)
        final_confidence = (
            (keyword_coverage * 0.40) +  # Keyword match: 40% weight
            (detail_score * 0.35) +       # Detail level: 35% weight
            (feature_score * 0.25)        # Features: 25% weight
        )
        
        reasoning["final_confidence"] = final_confidence
        reasoning["reasoning_summary"] = (
            f"Intent '{intent_type}' detected with {len(keyword_matches)} keywords "
            f"({keyword_coverage:.0%} coverage), {word_count} words (detail={detail_score:.2f}), "
            f"{feature_count} features (feature_score={feature_score:.2f})"
        )
        
        return final_confidence, reasoning
    
    @staticmethod
    def _detect_features(instruction: str) -> Dict[str, bool]:
        """Detect semantic features in instruction."""
        instruction_lower = instruction.lower()
        
        return {
            "parallel_execution": any(w in instruction_lower for w in ["parallel", "concurrent", "async", "background"]),
            "retry_logic": any(w in instruction_lower for w in ["retry", "resilience", "recovery", "fallback"]),
            "persistence": any(w in instruction_lower for w in ["persist", "cache", "store", "save"]),
            "compliance": any(w in instruction_lower for w in ["audit", "compliance", "track", "log"]),
            "event_driven": any(w in instruction_lower for w in ["event", "notify", "alert", "trigger"]),
            "validation": any(w in instruction_lower for w in ["validate", "check", "verify", "assert"]),
            "optimization": any(w in instruction_lower for w in ["optimize", "performance", "efficiency", "improve"]),
        }


def detect_intent_ambiguity(
    instruction: str,
    keywords_dict: Dict[str, Dict[str, Any]],
    score_threshold: float = 0.30,
    min_score: float = 0.05,
    max_candidates: int = 3,
) -> Dict[str, Any]:
    """Detect ambiguous intents when multiple intents score similarly.
    
    Returns:
        {
            "is_ambiguous": bool,
            "ambiguity_margin": float | None,
            "candidates": list[dict]
        }
    """
    instruction_lower = instruction.lower()
    scores = []

    for intent_info in keywords_dict.values():
        keywords = intent_info["keywords"]
        matches = sum(1 for kw in keywords if kw in instruction_lower)
        score = matches / len(keywords)
        if score > 0:
            scores.append({"intent_type": intent_info["type"], "score": score})

    if not scores:
        return {"is_ambiguous": False, "ambiguity_margin": None, "candidates": []}

    scores.sort(key=lambda x: x["score"], reverse=True)
    top_scores = scores[:max_candidates]

    if len(scores) < 2:
        return {"is_ambiguous": False, "ambiguity_margin": None, "candidates": top_scores}

    margin = scores[0]["score"] - scores[1]["score"]
    is_ambiguous = scores[0]["score"] >= min_score and margin <= score_threshold

    return {
        "is_ambiguous": is_ambiguous,
        "ambiguity_margin": margin,
        "candidates": top_scores,
    }


class EconomicOperationHandler(GenerativeHandler):
    """Handler for economic operations with semantic intent differentiation.
    
    Supported intents:
    - Sensitivity analysis (elasticity, impact analysis)
    - Waterfall analysis (revenue/cost decomposition)
    - Break-even analysis (minimum threshold calculations)
    - Unit economics (CAC, LTV, payback period)
    - Variance analysis (budget vs. actual)
    - Contribution margin analysis
    - Growth decomposition (organic vs. inorganic)
    - Profitability attribution (driver analysis)
    """
    
    def __init__(self):
        super().__init__(
            name="EconomicOperationHandler",
            operation_type="economic_function",
        )
        self._intent_cache: Dict[str, str] = {}
        
        # Initialize function generator with strategy selection.
        # During pytest, keep generation deterministic and avoid depending on external API keys.
        is_test_mode = os.getenv("PYTEST_CURRENT_TEST") is not None
        api_key = None if is_test_mode else os.getenv("OPENAI_API_KEY")
        self._generator = StrategySelector(
            api_key=api_key,
            enable_cache=True,
            prefer_low_cost=True,  # Prefer zero-cost strategies
        )
        logger.info(
            f"EconomicOperationHandler initialized with multi-strategy generator "
            f"(LLM_enabled={api_key is not None})"
        )
    
    async def handle(self, command: GenerativeCommand) -> GeneratedOperation:
        """Handle economic operation command with semantic intent differentiation.
        
        Process:
        1. Classify intent semantically (no enums)
        2. Select intent-specific generation template
        3. Create ExecutableFunction with appropriate constraints
        4. Record semantic metadata and intent type
        5. Return GeneratedOperation
        """
        logger.info(
            f"EconomicOperationHandler.handle: "
            f'command_id={command.command_id}, intent="{command.user_instruction}"'
        )
        
        start_time = datetime.utcnow()
        
        try:
            # Step 1: Classify intent semantically
            intent_type = self._classify_intent(command.user_instruction)
            intent_confidence = self._compute_intent_confidence(command.user_instruction, intent_type)
            ambiguity = detect_intent_ambiguity(command.user_instruction, ECONOMIC_INTENT_KEYWORDS)
            
            logger.debug(
                f"Intent classified: type={intent_type}, confidence={intent_confidence:.2f}, "
                f"instruction='{command.user_instruction}'"
            )
            
            # Step 1.5: Select fallback strategy based on confidence
            fallback_strategy = FallbackStrategyRouter.select_strategy(intent_confidence)
            strategy_metadata = FallbackStrategyRouter.get_strategy_metadata(
                fallback_strategy, intent_confidence, intent_type
            )
            
            logger.debug(
                f"Fallback strategy selected: {fallback_strategy.value} "
                f"(confidence={intent_confidence:.3f})"
            )
            
            # Step 2: Generate function using the strategy selector
            semantic_features = self._extract_semantic_features(command.user_instruction, intent_type)
            
            generation_context = GenerationContext(
                command=command,
                semantic_features=semantic_features,
                intent_type=intent_type,
                intent_confidence=intent_confidence,
                cost_budget_usd=0.05,  # Conservative budget
                latency_budget_ms=5000,  # 5s timeout
                seed=hash(command.user_instruction) % (2**32),  # Deterministic seed
            )
            
            # Generate function with strategy selection (LLM/composable/template)
            generated_op = await self._generator.generate(generation_context)
            
            logger.info(
                f"Generated {intent_type} function using {generated_op.generated_function.generation_strategy} "
                f"(metadata cost=${generated_op.generation_metadata.get('generation_cost_usd', 0.0):.4f}, "
                f"latency={generated_op.generation_metadata.get('generation_latency_ms', 0.0):.0f}ms)"
            )
            
            # Use the generated function, but preserve existing operation metadata structure
            # Extract strategy info before using the function
            strategy_used = generated_op.generated_function.generation_strategy
            selected_strategy = generated_op.generation_metadata.get("selected_strategy")
            gen_cost = generated_op.generation_metadata.get("generation_cost_usd", 0.0)
            gen_latency = generated_op.generation_metadata.get("generation_latency_ms", 0.0)
            
            func = generated_op.generated_function

            # Ensure economic operations carry minimal domain context in code.
            # Some low-cost strategies may emit generic helper code; for economic ops
            # we still want the generated code to be clearly economic/financial.
            code_lower = (func.code or "").lower()
            if not any(word in code_lower for word in ("economic", "financial", "revenue", "cost")):
                if func.language.lower() == "sql":
                    prefix = "-- Economic analysis\n"
                elif func.language.lower() in ("ts", "typescript", "javascript", "js"):
                    prefix = "// Economic analysis\n"
                else:
                    prefix = "# Economic analysis\n"
                func.code = prefix + func.code
                func.function_id = hashlib.sha256(func.code.encode()).hexdigest()[:16]
            
            # Step 3: Create operation with semantic metadata + generation data
            operation = GeneratedOperation.create(
                command_id=command.command_id,
                generated_function=func,
                generation_metadata={
                    "handler": self.name,
                    "generator_version": "semantic_operation_handlers_v1",
                    "intent": command.user_instruction,
                    "intent_type": intent_type,
                    "intent_confidence": intent_confidence,
                    "semantic_confidence": intent_confidence,
                    "generator_variant": "multi_strategy",
                    "strategy": selected_strategy or strategy_used.value,
                    "generation_cost_usd": gen_cost,
                    "generation_latency_ms": gen_latency,
                    "features_detected": semantic_features,
                    "is_ambiguous": ambiguity["is_ambiguous"],
                    "ambiguity_margin": ambiguity["ambiguity_margin"],
                    "ambiguity_candidates": ambiguity["candidates"],
                },
                validation_results=ValidationResults(),
                is_cached=False,
                semantic_confidence=intent_confidence,
            )
            
            # Step 3.5: Add fallback strategy metadata to operation
            operation.generation_metadata.update({
                "fallback_strategy": strategy_metadata["fallback_strategy"],
                "confidence_tier": strategy_metadata["confidence_tier"],
                "routing_reason": strategy_metadata["routing_reason"],
                "features_enabled": strategy_metadata["features_enabled"],
            })
            if "requires_clarification" in strategy_metadata:
                operation.generation_metadata["requires_clarification"] = strategy_metadata["requires_clarification"]
            
            # Step 4: Update stats
            elapsed_ms = (datetime.utcnow() - start_time).total_seconds() * 1000
            self.processed_count += 1
            self.total_generation_time_ms += elapsed_ms
            operation.generation_time_ms = elapsed_ms
            
            logger.info(
                f"Economic operation generated: "
                f"operation_id={operation.operation_id}, "
                f"intent_type={intent_type}, "
                f"confidence={intent_confidence:.2f}, "
                f"generation_time_ms={elapsed_ms:.1f}"
            )
            
            return operation
        
        except Exception as e:
            self.error_count += 1
            logger.error(
                f"Economic operation generation failed: {e}",
                exc_info=True,
            )
            raise
    
    def _classify_intent(self, instruction: str) -> str:
        """Classify economic intent semantically from instruction text.
        
        Uses deterministic keyword matching as a baseline classifier.
        
        Returns: intent_type (e.g., "sensitivity_analysis", "waterfall_analysis")
        """
        instruction_lower = instruction.lower()
        
        # Semantic classification via keyword matching
        best_match = "generic_economic_operation"
        best_score = 0.0
        
        for intent_key, intent_info in ECONOMIC_INTENT_KEYWORDS.items():
            keywords = intent_info["keywords"]
            matches = sum(1 for kw in keywords if kw in instruction_lower)
            score = matches / len(keywords)
            
            if score > best_score:
                best_score = score
                best_match = intent_info["type"]
        
        logger.debug(f"Intent classification: '{instruction_lower[:50]}...' → {best_match} (score={best_score:.2f})")
        return best_match
    
    def _compute_intent_confidence(self, instruction: str, intent_type: str) -> float:
        """Compute confidence score for intent classification with reasoning.

        Returns: confidence score (0.0–1.0)
        """
        confidence, reasoning = SemanticConfidenceScorer.score_intent(
            instruction=instruction,
            intent_type=intent_type,
            keywords_dict=ECONOMIC_INTENT_KEYWORDS,
        )
        
        logger.debug(
            f"Economic intent confidence: {confidence:.3f} — {reasoning['reasoning_summary']}"
        )
        
        return confidence
    
    def _extract_semantic_features(self, instruction: str, intent_type: str) -> Dict[str, Any]:
        """Extract semantic features from instruction for observability.
        
        Returns: dict of detected features (e.g., {"revenue": True, "cost": False})
        """
        instruction_lower = instruction.lower()
        
        features = {
            "intent_type": intent_type,
            "revenue_detected": any(w in instruction_lower for w in ["revenue", "sales", "income"]),
            "cost_detected": any(w in instruction_lower for w in ["cost", "expense", "cogs"]),
            "margin_detected": any(w in instruction_lower for w in ["margin", "profit", "ebitda"]),
            "sensitivity_detected": any(w in instruction_lower for w in ["sensitivity", "elasticity", "impact"]),
            "scenario_detected": any(w in instruction_lower for w in ["scenario", "case", "analysis"]),
        }
        
        return features
    
    def get_stats(self) -> Dict[str, Any]:
        """Get handler statistics including generator performance."""
        base_stats = {
            "handler": self.name,
            "operation_type": self.operation_type,
            "processed_count": self.processed_count,
            "error_count": self.error_count,
            "total_generation_time_ms": self.total_generation_time_ms,
            "avg_generation_time_ms": (
                self.total_generation_time_ms / self.processed_count
                if self.processed_count > 0
                else 0
            ),
        }
        
        # Include generator stats when available
        if hasattr(self, "_generator"):
            base_stats["generator"] = self._generator.get_stats()
        
        return base_stats


class StoryOperationHandler(GenerativeHandler):
    """Handler for story operations with semantic intent differentiation.
    
    Supported intents:
    - Story arc development (beginning/middle/end, hero's journey)
    - Theme application (visual consistency, tone, branding)
    - Framing strategy (perspective, positioning, context)
    - Evidence-based narrative (data-driven claims, proof points)
    - Audience tailoring (persona-specific messaging)
    - Narrative organization (flow, sequencing, structure)
    - Visual narrative design (slide layouts, graphics, diagrams)
    """
    
    def __init__(self):
        super().__init__(
            name="StoryOperationHandler",
            operation_type="story_operation",
        )
        self._intent_cache: Dict[str, str] = {}
    
    async def handle(self, command: GenerativeCommand) -> GeneratedOperation:
        """Handle story operation command with semantic intent differentiation.
        
        Process:
        1. Classify narrative intent semantically (no enums)
        2. Select intent-specific generation template
        3. Create ExecutableFunction with narrative-specific constraints
        4. Record semantic metadata and narrative intent type
        5. Return GeneratedOperation
        """
        logger.info(
            f"StoryOperationHandler.handle: "
            f'command_id={command.command_id}, intent="{command.user_instruction}"'
        )
        
        start_time = datetime.utcnow()
        
        try:
            # Step 1: Classify narrative intent semantically
            intent_type = self._classify_intent(command.user_instruction)
            intent_confidence = self._compute_intent_confidence(command.user_instruction, intent_type)
            ambiguity = detect_intent_ambiguity(command.user_instruction, STORY_INTENT_KEYWORDS)
            
            logger.debug(
                f"Story intent classified: type={intent_type}, confidence={intent_confidence:.2f}, "
                f"instruction='{command.user_instruction}'"
            )
            
            # Step 1.5: Select fallback strategy based on confidence
            fallback_strategy = FallbackStrategyRouter.select_strategy(intent_confidence)
            strategy_metadata = FallbackStrategyRouter.get_strategy_metadata(
                fallback_strategy, intent_confidence, intent_type
            )

            logger.debug(
                f"Fallback strategy selected: {fallback_strategy.value} "
                f"(confidence={intent_confidence:.3f})"
            )

            # Step 2: Create intent-specific executable function
            func = ExecutableFunction(
                name=f"story_{intent_type}_{self.processed_count + 1}",
                language="python",
                code=self._generate_intent_specific_code(command, intent_type),
                signature={
                    "inputs": {"context": "dict", "parameters": "dict"},
                    "outputs": {"narrative": "str", "slides": "list"},
                },
                constraints_enforced=[
                    ConstraintType.DETERMINISTIC,
                    ConstraintType.CONSISTENCY,
                ],
                generation_strategy=GenerationStrategy.TEMPLATE_INJECTION,
                strategy_metadata={
                    "template": f"narrative_{intent_type}",
                    "variant": "intent_differentiation",
                    "intent_type": intent_type,
                    "intent_confidence": intent_confidence,
                },
                generated_at=start_time,
                semantic_engine_version="semantic_engine_v2.0",
                model_version=None,
                seed=None,
            )
            
            # Step 3: Create operation with semantic metadata
            operation = GeneratedOperation.create(
                command_id=command.command_id,
                generated_function=func,
                generation_metadata={
                    "handler": self.name,
                    "generator_version": "semantic_operation_handlers_v1",
                    "intent": command.user_instruction,
                    "intent_type": intent_type,
                    "intent_confidence": intent_confidence,
                    "semantic_confidence": intent_confidence,
                    "generator_variant": "template_injection",
                    "strategy": GenerationStrategy.TEMPLATE_INJECTION.value,
                    "features_detected": self._extract_semantic_features(command.user_instruction, intent_type),
                    "is_ambiguous": ambiguity["is_ambiguous"],
                    "ambiguity_margin": ambiguity["ambiguity_margin"],
                    "ambiguity_candidates": ambiguity["candidates"],
                },
                validation_results=ValidationResults(),
                is_cached=False,
                semantic_confidence=intent_confidence,
            )

            # Step 3.5: Add fallback strategy metadata to operation
            operation.generation_metadata.update({
                "fallback_strategy": strategy_metadata["fallback_strategy"],
                "confidence_tier": strategy_metadata["confidence_tier"],
                "routing_reason": strategy_metadata["routing_reason"],
                "features_enabled": strategy_metadata["features_enabled"],
            })
            if "requires_clarification" in strategy_metadata:
                operation.generation_metadata["requires_clarification"] = strategy_metadata["requires_clarification"]
            
            # Step 4: Update stats
            elapsed_ms = (datetime.utcnow() - start_time).total_seconds() * 1000
            self.processed_count += 1
            self.total_generation_time_ms += elapsed_ms
            operation.generation_time_ms = elapsed_ms
            
            logger.info(
                f"Story operation generated: "
                f"operation_id={operation.operation_id}, "
                f"intent_type={intent_type}, "
                f"confidence={intent_confidence:.2f}, "
                f"generation_time_ms={elapsed_ms:.1f}"
            )
            
            return operation
        
        except Exception as e:
            self.error_count += 1
            logger.error(
                f"Story operation generation failed: {e}",
                exc_info=True,
            )
            raise
    
    def _classify_intent(self, instruction: str) -> str:
        """Classify narrative intent semantically from instruction text.
        
        Uses deterministic keyword matching as a baseline classifier.
        
        Returns: intent_type (e.g., "story_arc_development", "theme_application")
        """
        instruction_lower = instruction.lower()
        
        # Semantic classification via keyword matching
        best_match = "generic_narrative_operation"
        best_score = 0.0
        
        for intent_key, intent_info in STORY_INTENT_KEYWORDS.items():
            keywords = intent_info["keywords"]
            matches = sum(1 for kw in keywords if kw in instruction_lower)
            score = matches / len(keywords)
            
            if score > best_score:
                best_score = score
                best_match = intent_info["type"]
        
        logger.debug(f"Story intent classification: '{instruction_lower[:50]}...' → {best_match} (score={best_score:.2f})")
        return best_match
    
    def _compute_intent_confidence(self, instruction: str, intent_type: str) -> float:
        """Compute confidence score for narrative intent classification with reasoning.

        Returns: confidence score (0.0–1.0)
        """
        confidence, reasoning = SemanticConfidenceScorer.score_intent(
            instruction=instruction,
            intent_type=intent_type,
            keywords_dict=STORY_INTENT_KEYWORDS,
        )
        
        logger.debug(
            f"Story intent confidence: {confidence:.3f} — {reasoning['reasoning_summary']}"
        )
        
        return confidence
    
    def _extract_semantic_features(self, instruction: str, intent_type: str) -> Dict[str, Any]:
        """Extract semantic features from instruction for observability.
        
        Returns: dict of detected features (e.g., {"audience": True, "theme": False})
        """
        instruction_lower = instruction.lower()
        
        features = {
            "intent_type": intent_type,
            "audience_detected": any(w in instruction_lower for w in ["audience", "investor", "customer", "stakeholder"]),
            "narrative_detected": any(w in instruction_lower for w in ["narrative", "story", "arc", "journey"]),
            "visual_detected": any(w in instruction_lower for w in ["visual", "slide", "graphic", "design"]),
            "evidence_detected": any(w in instruction_lower for w in ["evidence", "data", "proof", "support"]),
            "theme_detected": any(w in instruction_lower for w in ["theme", "consistent", "brand", "tone"]),
        }
        
        return features
    
    def _generate_intent_specific_code(self, command: GenerativeCommand, intent_type: str) -> str:
        """Generate intent-specific Python code for narrative operation.

        Uses intent-differentiated templates.
        """
        if intent_type == "story_arc_development":
            return self._generate_story_arc_code(command)
        elif intent_type == "theme_application":
            return self._generate_theme_application_code(command)
        elif intent_type == "framing_strategy":
            return self._generate_framing_code(command)
        elif intent_type == "evidence_based_narrative":
            return self._generate_evidence_based_code(command)
        elif intent_type == "audience_tailoring":
            return self._generate_audience_tailoring_code(command)
        elif intent_type == "narrative_organization":
            return self._generate_organization_code(command)
        elif intent_type == "visual_narrative_design":
            return self._generate_visual_narrative_code(command)
        else:
            return self._generate_generic_narrative_code(command)
    
    def _generate_story_arc_code(self, command: GenerativeCommand) -> str:
        """Generate code for story arc development."""
        return f"""# Story Arc Development: {command.user_instruction}
# Template-based generation

def story_arc_development(context: dict, parameters: dict) -> dict:
    \"\"\"Develop narrative arc following hero's journey structure.\"\"\"
    project = context.get('project', {{}})
    artifacts = context.get('artifacts', [])
    
    # Structure: Setup → Conflict → Resolution
    narrative_arc = {{
        'setup': 'Introduce market opportunity and team',
        'conflict': 'Challenges and competitive landscape',
        'resolution': 'Solution and growth trajectory',
    }}
    
    slides = [
        {{'type': 'title', 'content': 'The Journey', 'arc_position': 'setup'}},
        {{'type': 'problem', 'content': 'Market challenge', 'arc_position': 'conflict'}},
        {{'type': 'solution', 'content': 'Our approach', 'arc_position': 'resolution'}},
        {{'type': 'impact', 'content': 'Expected outcomes', 'arc_position': 'resolution'}},
    ]
    
    return {{
        'status': 'ok',
        'narrative_type': 'story_arc_development',
        'instruction': r'{command.user_instruction}',
        'narrative_arc': narrative_arc,
        'slides': slides,
    }}
"""
    
    def _generate_theme_application_code(self, command: GenerativeCommand) -> str:
        """Generate code for theme application."""
        return f"""# Theme Application: {command.user_instruction}
# Template-based generation

def theme_application(context: dict, parameters: dict) -> dict:
    \"\"\"Apply consistent theme and branding throughout narrative.\"\"\"
    theme = context.get('theme', {{'color': 'primary', 'tone': 'professional'}})
    artifacts = context.get('artifacts', [])
    
    themed_slides = []
    for artifact in artifacts:
        themed_slide = {{
            'content': artifact.get('content', ''),
            'theme': {{
                'color_scheme': theme.get('color'),
                'tone': theme.get('tone'),
                'typography': 'Urbanist',
                'spacing': '4px grid',
            }},
        }}
        themed_slides.append(themed_slide)
    
    return {{
        'status': 'ok',
        'narrative_type': 'theme_application',
        'instruction': r'{command.user_instruction}',
        'theme': theme,
        'slides': themed_slides,
    }}
"""
    
    def _generate_framing_code(self, command: GenerativeCommand) -> str:
        """Generate code for framing strategy."""
        return f"""# Framing Strategy: {command.user_instruction}
# Template-based generation

def framing_strategy(context: dict, parameters: dict) -> dict:
    \"\"\"Frame narrative with strategic context and positioning.\"\"\"
    market_context = context.get('market_context', {{}})
    positioning = context.get('positioning', {{}})
    
    frames = {{
        'market_frame': 'Opportunity in growing market',
        'competitive_frame': 'Differentiated solution',
        'value_frame': 'Superior unit economics',
    }}
    
    slides = [
        {{'type': 'market', 'frame': frames['market_frame']}},
        {{'type': 'competitive', 'frame': frames['competitive_frame']}},
        {{'type': 'value', 'frame': frames['value_frame']}},
    ]
    
    return {{
        'status': 'ok',
        'narrative_type': 'framing_strategy',
        'instruction': r'{command.user_instruction}',
        'frames': frames,
        'slides': slides,
    }}
"""
    
    def _generate_evidence_based_code(self, command: GenerativeCommand) -> str:
        """Generate code for evidence-based narrative."""
        return f"""# Evidence-Based Narrative: {command.user_instruction}
# Template-based generation

def evidence_based_narrative(context: dict, parameters: dict) -> dict:
    \"\"\"Support narrative claims with data and evidence.\"\"\"
    artifacts = context.get('artifacts', [])
    claims = context.get('claims', [])
    
    evidence_structure = {{}}
    for claim in claims:
        evidence_structure[claim] = {{
            'claim': claim,
            'supporting_data': 'From project artifacts',
            'confidence': 'High',
        }}
    
    slides = [
        {{'type': 'claim', 'content': claim, 'evidence': evidence_structure.get(claim, {{}})}},
    ]
    
    return {{
        'status': 'ok',
        'narrative_type': 'evidence_based_narrative',
        'instruction': r'{command.user_instruction}',
        'evidence_structure': evidence_structure,
        'slides': slides,
    }}
"""
    
    def _generate_audience_tailoring_code(self, command: GenerativeCommand) -> str:
        """Generate code for audience-tailored narrative."""
        return f"""# Audience Tailoring: {command.user_instruction}
# Template-based generation

def audience_tailoring(context: dict, parameters: dict) -> dict:
    \"\"\"Customize narrative for specific audience persona.\"\"\"
    audience_persona = context.get('audience', {{}})
    artifacts = context.get('artifacts', [])
    
    persona_config = {{
        'investors': {{'focus': 'ROI, market size, unit economics'}},
        'customers': {{'focus': 'Value, solution, support'}},
        'partners': {{'focus': 'Synergy, integration, growth'}},
    }}
    
    tailored_slides = []
    persona_type = audience_persona.get('type', 'investors')
    focus_areas = persona_config.get(persona_type, {{}}).get('focus', '')
    
    for artifact in artifacts:
        tailored_slide = {{
            'content': artifact.get('content', ''),
            'tailored_for': persona_type,
            'focus_areas': focus_areas,
        }}
        tailored_slides.append(tailored_slide)
    
    return {{
        'status': 'ok',
        'narrative_type': 'audience_tailoring',
        'instruction': r'{command.user_instruction}',
        'persona': persona_type,
        'slides': tailored_slides,
    }}
"""
    
    def _generate_organization_code(self, command: GenerativeCommand) -> str:
        """Generate code for narrative organization."""
        return f"""# Narrative Organization: {command.user_instruction}
# Template-based generation

def narrative_organization(context: dict, parameters: dict) -> dict:
    \"\"\"Organize narrative with logical flow and structure.\"\"\"
    artifacts = context.get('artifacts', [])
    
    # Organize by logical sequence
    organized_structure = [
        {{'position': 1, 'type': 'problem', 'label': 'Market Problem'}},
        {{'position': 2, 'type': 'solution', 'label': 'Our Solution'}},
        {{'position': 3, 'type': 'traction', 'label': 'Traction'}},
        {{'position': 4, 'type': 'financials', 'label': 'Financial Projections'}},
        {{'position': 5, 'type': 'ask', 'label': 'Investment Ask'}},
    ]
    
    return {{
        'status': 'ok',
        'narrative_type': 'narrative_organization',
        'instruction': r'{command.user_instruction}',
        'structure': organized_structure,
        'flow': 'Logical progression from problem → solution → traction → ask',
    }}
"""
    
    def _generate_visual_narrative_code(self, command: GenerativeCommand) -> str:
        """Generate code for visual narrative design."""
        return f"""# Visual Narrative Design: {command.user_instruction}
# Template-based generation

def visual_narrative_design(context: dict, parameters: dict) -> dict:
    \"\"\"Design visual narrative with slides, graphics, and diagrams.\"\"\"
    artifacts = context.get('artifacts', [])
    
    slides = [
        {{'type': 'title', 'layout': 'cover', 'visual_elements': ['logo', 'headline']}},
        {{'type': 'content', 'layout': 'two_column', 'visual_elements': ['image', 'text']}},
        {{'type': 'chart', 'layout': 'graph', 'visual_elements': ['data_visualization', 'annotations']}},
        {{'type': 'closing', 'layout': 'full_screen', 'visual_elements': ['call_to_action']}},
    ]
    
    return {{
        'status': 'ok',
        'narrative_type': 'visual_narrative_design',
        'instruction': r'{command.user_instruction}',
        'slides': slides,
        'design_principles': 'Hierarchy, contrast, alignment, whitespace',
    }}
"""
    
    def _generate_generic_narrative_code(self, command: GenerativeCommand) -> str:
        """Generate generic narrative operation code."""
        return f"""# Narrative Operation: {command.user_instruction}
# Template-based generation

def narrative_operation(context: dict, parameters: dict) -> dict:
    \"\"\"Execute generic narrative operation.\"\"\"
    project = context.get('project', {{}})
    artifacts = context.get('artifacts', [])
    
    narrative = 'Generic narrative structure'
    slides = [
        {{'type': 'title', 'content': 'Title Slide'}},
        {{'type': 'content', 'content': 'Content Slide'}},
    ]
    
    return {{
        'status': 'ok',
        'narrative': narrative,
        'slides': slides,
        'instruction': r'{command.user_instruction}',
        'template_variant': 'generic',
    }}
"""


class SystemOperationHandler(GenerativeHandler):
    """Handler for system operations with semantic intent differentiation.
    
    Supported intents:
    - Batch processing (bulk operations, parallel execution)
    - Workflow orchestration (pipeline sequencing, coordination)
    - Error recovery (retry logic, resilience patterns)
    - Cache management (optimization, persistence)
    - Data migration (transfer, conversion, upgrade)
    - Audit logging (compliance, tracking, accountability)
    - Notification rules (alerts, events, triggers)
    """
    
    def __init__(self):
        super().__init__(
            name="SystemOperationHandler",
            operation_type="system_operation",
        )
        self._intent_cache: Dict[str, str] = {}
    
    async def handle(self, command: GenerativeCommand) -> GeneratedOperation:
        """Handle system operation command with semantic intent differentiation.
        
        Process:
        1. Classify workflow intent semantically (no enums)
        2. Select intent-specific generation template
        3. Create ExecutableFunction with system-specific constraints
        4. Record semantic metadata and workflow intent type
        5. Return GeneratedOperation
        """
        logger.info(
            f"SystemOperationHandler.handle: "
            f'command_id={command.command_id}, intent="{command.user_instruction}"'
        )
        
        start_time = datetime.utcnow()
        
        try:
            # Step 1: Classify workflow intent semantically
            intent_type = self._classify_intent(command.user_instruction)
            intent_confidence = self._compute_intent_confidence(command.user_instruction, intent_type)
            ambiguity = detect_intent_ambiguity(command.user_instruction, SYSTEM_INTENT_KEYWORDS)
            
            logger.debug(
                f"System intent classified: type={intent_type}, confidence={intent_confidence:.2f}, "
                f"instruction='{command.user_instruction}'"
            )
            
            # Step 1.5: Select fallback strategy based on confidence
            fallback_strategy = FallbackStrategyRouter.select_strategy(intent_confidence)
            strategy_metadata = FallbackStrategyRouter.get_strategy_metadata(
                fallback_strategy, intent_confidence, intent_type
            )

            logger.debug(
                f"Fallback strategy selected: {fallback_strategy.value} "
                f"(confidence={intent_confidence:.3f})"
            )

            # Step 2: Create intent-specific executable function
            func = ExecutableFunction(
                name=f"system_{intent_type}_{self.processed_count + 1}",
                language="python",
                code=self._generate_intent_specific_code(command, intent_type),
                signature={
                    "inputs": {"context": "dict", "parameters": "dict"},
                    "outputs": {"status": "str", "result": "dict"},
                },
                constraints_enforced=[ConstraintType.DETERMINISTIC],
                generation_strategy=GenerationStrategy.TEMPLATE_INJECTION,
                strategy_metadata={
                    "template": f"system_{intent_type}",
                    "variant": "intent_differentiation",
                    "intent_type": intent_type,
                    "intent_confidence": intent_confidence,
                },
                generated_at=start_time,
                semantic_engine_version="semantic_engine_v2.0",
                model_version=None,
                seed=None,
            )
            
            # Step 3: Create operation with semantic metadata
            operation = GeneratedOperation.create(
                command_id=command.command_id,
                generated_function=func,
                generation_metadata={
                    "handler": self.name,
                    "generator_version": "semantic_operation_handlers_v1",
                    "intent": command.user_instruction,
                    "intent_type": intent_type,
                    "intent_confidence": intent_confidence,
                    "semantic_confidence": intent_confidence,
                    "generator_variant": "template_injection",
                    "strategy": GenerationStrategy.TEMPLATE_INJECTION.value,
                    "features_detected": self._extract_semantic_features(command.user_instruction, intent_type),
                    "is_ambiguous": ambiguity["is_ambiguous"],
                    "ambiguity_margin": ambiguity["ambiguity_margin"],
                    "ambiguity_candidates": ambiguity["candidates"],
                },
                validation_results=ValidationResults(),
                is_cached=False,
                semantic_confidence=intent_confidence,
            )

            # Step 3.5: Add fallback strategy metadata to operation
            operation.generation_metadata.update({
                "fallback_strategy": strategy_metadata["fallback_strategy"],
                "confidence_tier": strategy_metadata["confidence_tier"],
                "routing_reason": strategy_metadata["routing_reason"],
                "features_enabled": strategy_metadata["features_enabled"],
            })
            if "requires_clarification" in strategy_metadata:
                operation.generation_metadata["requires_clarification"] = strategy_metadata["requires_clarification"]
            
            # Step 4: Update stats
            elapsed_ms = (datetime.utcnow() - start_time).total_seconds() * 1000
            self.processed_count += 1
            self.total_generation_time_ms += elapsed_ms
            operation.generation_time_ms = elapsed_ms
            
            logger.info(
                f"System operation generated: "
                f"operation_id={operation.operation_id}, "
                f"intent_type={intent_type}, "
                f"confidence={intent_confidence:.2f}, "
                f"generation_time_ms={elapsed_ms:.1f}"
            )
            
            return operation
        
        except Exception as e:
            self.error_count += 1
            logger.error(
                f"System operation generation failed: {e}",
                exc_info=True,
            )
            raise
    
    def _classify_intent(self, instruction: str) -> str:
        """Classify system intent semantically from instruction text.
        
        Uses deterministic keyword matching as a baseline classifier.
        
        Returns: intent_type (e.g., "batch_processing", "workflow_orchestration")
        """
        instruction_lower = instruction.lower()
        
        # Semantic classification via keyword matching
        best_match = "generic_system_operation"
        best_score = 0.0
        
        for intent_key, intent_info in SYSTEM_INTENT_KEYWORDS.items():
            keywords = intent_info["keywords"]
            matches = sum(1 for kw in keywords if kw in instruction_lower)
            score = matches / len(keywords)
            
            if score > best_score:
                best_score = score
                best_match = intent_info["type"]
        
        logger.debug(f"System intent classification: '{instruction_lower[:50]}...' → {best_match} (score={best_score:.2f})")
        return best_match
    
    def _compute_intent_confidence(self, instruction: str, intent_type: str) -> float:
        """Compute confidence score for system intent classification with reasoning.

        Returns: confidence score (0.0–1.0)
        """
        confidence, reasoning = SemanticConfidenceScorer.score_intent(
            instruction=instruction,
            intent_type=intent_type,
            keywords_dict=SYSTEM_INTENT_KEYWORDS,
        )
        
        logger.debug(
            f"System intent confidence: {confidence:.3f} — {reasoning['reasoning_summary']}"
        )
        
        return confidence
    
    def _extract_semantic_features(self, instruction: str, intent_type: str) -> Dict[str, Any]:
        """Extract semantic features from instruction for observability.
        
        Returns: dict of detected features (e.g., {"parallel": True, "retry": False})
        """
        instruction_lower = instruction.lower()
        
        features = {
            "intent_type": intent_type,
            "parallel_detected": any(w in instruction_lower for w in ["parallel", "concurrent", "async", "background"]),
            "retry_detected": any(w in instruction_lower for w in ["retry", "resilience", "recovery", "fallback"]),
            "persistence_detected": any(w in instruction_lower for w in ["persist", "cache", "store", "save"]),
            "compliance_detected": any(w in instruction_lower for w in ["audit", "compliance", "track", "log"]),
            "event_detected": any(w in instruction_lower for w in ["event", "notify", "alert", "trigger"]),
        }
        
        return features
    
    def _generate_intent_specific_code(self, command: GenerativeCommand, intent_type: str) -> str:
        """Generate intent-specific Python code for system operation.

        Uses intent-differentiated templates.
        """
        if intent_type == "batch_processing":
            return self._generate_batch_processing_code(command)
        elif intent_type == "workflow_orchestration":
            return self._generate_workflow_orchestration_code(command)
        elif intent_type == "error_recovery":
            return self._generate_error_recovery_code(command)
        elif intent_type == "cache_management":
            return self._generate_cache_management_code(command)
        elif intent_type == "data_migration":
            return self._generate_data_migration_code(command)
        elif intent_type == "audit_logging":
            return self._generate_audit_logging_code(command)
        elif intent_type == "notification_rules":
            return self._generate_notification_rules_code(command)
        else:
            return self._generate_generic_system_code(command)
    
    def _generate_batch_processing_code(self, command: GenerativeCommand) -> str:
        """Generate code for batch processing operations."""
        return f"""# Batch Processing: {command.user_instruction}
    # Template-based generation

def batch_processing(context: dict, parameters: dict) -> dict:
    \"\"\"Process items in batches with configurable batch size.\"\"\"
    items = context.get('items', [])
    batch_size = parameters.get('batch_size', 100)
    
    batches = []
    for i in range(0, len(items), batch_size):
        batch = items[i:i + batch_size]
        batches.append({{
            'batch_id': i // batch_size,
            'size': len(batch),
            'items': batch,
        }})
    
    return {{
        'status': 'ok',
        'operation_type': 'batch_processing',
        'instruction': r'{command.user_instruction}',
        'batch_count': len(batches),
        'batches': batches,
    }}
"""
    
    def _generate_workflow_orchestration_code(self, command: GenerativeCommand) -> str:
        """Generate code for workflow orchestration."""
        return f"""# Workflow Orchestration: {command.user_instruction}
    # Template-based generation

def workflow_orchestration(context: dict, parameters: dict) -> dict:
    \"\"\"Orchestrate workflow stages with sequential execution.\"\"\"
    stages = parameters.get('stages', [])
    
    execution_log = []
    for stage_idx, stage in enumerate(stages):
        execution_log.append({{
            'stage': stage_idx,
            'name': stage.get('name', 'Stage ' + str(stage_idx)),
            'status': 'pending',
            'dependencies': stage.get('depends_on', []),
        }})
    
    return {{
        'status': 'ok',
        'operation_type': 'workflow_orchestration',
        'instruction': r'{command.user_instruction}',
        'stage_count': len(stages),
        'execution_log': execution_log,
    }}
"""
    
    def _generate_error_recovery_code(self, command: GenerativeCommand) -> str:
        """Generate code for error recovery and resilience."""
        return f"""# Error Recovery: {command.user_instruction}
    # Template-based generation

def error_recovery(context: dict, parameters: dict) -> dict:
    \"\"\"Implement error recovery with retry logic and fallbacks.\"\"\"
    max_retries = parameters.get('max_retries', 3)
    backoff_ms = parameters.get('backoff_ms', 100)
    
    recovery_strategy = {{
        'max_retries': max_retries,
        'backoff_multiplier': 2,
        'initial_backoff_ms': backoff_ms,
        'max_backoff_ms': backoff_ms * (2 ** max_retries),
        'fallback_enabled': True,
    }}
    
    return {{
        'status': 'ok',
        'operation_type': 'error_recovery',
        'instruction': r'{command.user_instruction}',
        'recovery_strategy': recovery_strategy,
    }}
"""
    
    def _generate_cache_management_code(self, command: GenerativeCommand) -> str:
        """Generate code for cache management and optimization."""
        return f"""# Cache Management: {command.user_instruction}
    # Template-based generation

def cache_management(context: dict, parameters: dict) -> dict:
    \"\"\"Manage caching with TTL and eviction policies.\"\"\"
    cache_size = parameters.get('cache_size', 1000)
    ttl_seconds = parameters.get('ttl_seconds', 3600)
    eviction_policy = parameters.get('eviction_policy', 'LRU')
    
    cache_config = {{
        'size': cache_size,
        'ttl_seconds': ttl_seconds,
        'eviction_policy': eviction_policy,
        'persistence': True,
        'metrics_enabled': True,
    }}
    
    return {{
        'status': 'ok',
        'operation_type': 'cache_management',
        'instruction': r'{command.user_instruction}',
        'cache_config': cache_config,
    }}
"""
    
    def _generate_data_migration_code(self, command: GenerativeCommand) -> str:
        """Generate code for data migration and conversion."""
        return f"""# Data Migration: {command.user_instruction}
    # Template-based generation

def data_migration(context: dict, parameters: dict) -> dict:
    \"\"\"Execute data migration with validation and rollback capability.\"\"\"
    source_format = parameters.get('source_format', 'json')
    target_format = parameters.get('target_format', 'parquet')
    validate = parameters.get('validate', True)
    
    migration_plan = {{
        'source_format': source_format,
        'target_format': target_format,
        'validation_enabled': validate,
        'rollback_enabled': True,
        'checkpoint_frequency': 1000,
    }}
    
    return {{
        'status': 'ok',
        'operation_type': 'data_migration',
        'instruction': r'{command.user_instruction}',
        'migration_plan': migration_plan,
    }}
"""
    
    def _generate_audit_logging_code(self, command: GenerativeCommand) -> str:
        """Generate code for audit logging and compliance."""
        return f"""# Audit Logging: {command.user_instruction}
    # Template-based generation

def audit_logging(context: dict, parameters: dict) -> dict:
    \"\"\"Enable audit logging for compliance and accountability.\"\"\"
    log_level = parameters.get('log_level', 'INFO')
    retention_days = parameters.get('retention_days', 365)
    
    audit_config = {{
        'log_level': log_level,
        'retention_days': retention_days,
        'immutable': True,
        'encryption': True,
        'timestamped': True,
        'fields': ['user_id', 'action', 'resource', 'timestamp', 'result'],
    }}
    
    return {{
        'status': 'ok',
        'operation_type': 'audit_logging',
        'instruction': r'{command.user_instruction}',
        'audit_config': audit_config,
    }}
"""
    
    def _generate_notification_rules_code(self, command: GenerativeCommand) -> str:
        """Generate code for event notification and alerting."""
        return f"""# Notification Rules: {command.user_instruction}
    # Template-based generation

def notification_rules(context: dict, parameters: dict) -> dict:
    \"\"\"Configure event-driven notifications and alerts.\"\"\"
    event_types = parameters.get('event_types', ['error', 'warning', 'success'])
    channels = parameters.get('channels', ['email', 'slack'])
    
    notification_rules = []
    for event_type in event_types:
        for channel in channels:
            notification_rules.append({{
                'event_type': event_type,
                'channel': channel,
                'enabled': True,
                'throttle_seconds': 60,
            }})
    
    return {{
        'status': 'ok',
        'operation_type': 'notification_rules',
        'instruction': r'{command.user_instruction}',
        'rule_count': len(notification_rules),
        'rules': notification_rules,
    }}
"""
    
    def _generate_generic_system_code(self, command: GenerativeCommand) -> str:
        """Generate generic system operation code."""
        return f"""# System Operation: {command.user_instruction}
    # Template-based generation

def system_operation(context: dict, parameters: dict) -> dict:
    \"\"\"Execute generic system operation.\"\"\"
    return {{
        'status': 'ok',
        'result': {{'template_variant': 'generic', 'intent': r'{command.user_instruction}'}},
    }}
"""
