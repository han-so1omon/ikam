"""
SemanticEngine - Main orchestration layer for semantic evaluation.

This module provides the core dispatch mechanism that coordinates:
- Intent classification (Task 2.2)
- Evaluator registry queries (Task 2.3)
- Semantic embeddings for similarity (Task 2.1)
- Generative operation creation for novel intents

Architecture:
  User intent → SemanticEngine
    ↓
    ├─→ IntentClassifier (high-level categorization)
    ├─→ EvaluatorRegistry (capability-based matching)
    ├─→ SemanticEmbeddings (similarity scoring)
    └─→ LLM generation (novel operations)

Critical Principle (from AGENTS.md):
  "Semantic evaluation is a mandatory, always-available core feature."
  Generative operations are ALWAYS enabled - not a fallback, but core functionality.
"""

from dataclasses import dataclass
from typing import Dict, Any, List, Optional, Tuple
import logging

from modelado.intent_classifier import IntentClassifier, IntentClass, IntentClassificationResult
from modelado.semantic_evaluators import EvaluatorRegistry, EvaluationResult, create_default_registry
from modelado.semantic_embeddings import SemanticEmbeddings

logger = logging.getLogger(__name__)


@dataclass
class SemanticEvaluationResult:
    """Complete semantic evaluation result."""
    
    intent: str
    intent_class: IntentClass
    intent_confidence: float
    intent_features: List[str]
    
    # Best evaluator match
    evaluator_name: Optional[str]
    evaluator_confidence: float
    can_handle: bool
    reasoning: str
    
    # All evaluator results (for observability)
    all_evaluations: List[EvaluationResult]
    
    # Semantic features detected
    semantic_features: Dict[str, bool]
    
    # Capability metadata from selected evaluator
    capability_metadata: Dict[str, Any]
    
    # Generation info (true when LLM generated a novel operation)
    used_generation: bool
    generation_reason: Optional[str]


class SemanticEngine:
    """
    Main semantic evaluation engine.
    
    Orchestrates intent classification, evaluator matching, and generative operation
    creation to provide completely generative operation selection without hardcoded types.
    Generative operations are ALWAYS enabled per AGENTS.md constitution.
    
    Usage:
        engine = SemanticEngine()
        result = await engine.evaluate("Correlate revenue with market size")
        
        if result.can_handle:
            # Use result.evaluator_name and result.capability_metadata
            # to generate the appropriate function
            ...
    """
    
    def __init__(
        self,
        intent_classifier: Optional[IntentClassifier] = None,
        evaluator_registry: Optional[EvaluatorRegistry] = None,
        embeddings: Optional[SemanticEmbeddings] = None,
        min_confidence: float = 0.5,
    ):
        """
        Initialize SemanticEngine.
        
        Generative operations are ALWAYS enabled per AGENTS.md constitution.
        There is no option to disable them - they are core functionality.
        
        Args:
            intent_classifier: Intent classifier (creates default if None)
            evaluator_registry: Evaluator registry (creates default if None)
            embeddings: Semantic embeddings (creates default if None)
            min_confidence: Minimum confidence to accept an evaluator (default 0.5)
        """
        if intent_classifier is None:
            import os
            api_key = os.getenv("OPENAI_API_KEY", "")
            intent_classifier = IntentClassifier(openai_api_key=api_key)
        
        self.intent_classifier = intent_classifier
        self.evaluator_registry = evaluator_registry or create_default_registry()
        
        if embeddings is None:
            import os
            api_key = os.getenv("OPENAI_API_KEY", "")
            embeddings = SemanticEmbeddings(openai_api_key=api_key)
        self.embeddings = embeddings
        
        self.min_confidence = min_confidence

        # Log initialization (get_all is sync method)
        evaluators = self.evaluator_registry.get_all()
        logger.info(
            "SemanticEngine initialized: %d evaluators, min_confidence=%.2f, generative_ops=always_enabled",
            len(evaluators),
            min_confidence,
        )

    async def evaluate(self, intent: str, context: Optional[Dict[str, Any]] = None) -> SemanticEvaluationResult:
        """
        Evaluate intent using semantic matching.
        
        Steps:
        1. Classify intent at high level (economic/story/system/unknown)
        2. Query all evaluators in parallel
        3. Select best evaluator by confidence
        4. Fallback to LLM if no evaluator meets threshold
        
        Args:
            intent: User's natural language intent
            context: Optional context (project, artifacts, etc.)
        
        Returns:
            SemanticEvaluationResult with best evaluator match and metadata
        """
        context = context or {}
        
        logger.info("SemanticEngine.evaluate: intent='%s'", intent)
        
        # Step 1: High-level intent classification
        intent_result = await self.intent_classifier.classify(intent)
        
        logger.debug(
            "Intent classified: class=%s, confidence=%.2f, features=%s",
            intent_result.predicted_class.value,
            intent_result.confidence,
            intent_result.detected_features,
        )
        
        # Step 2: Query all evaluators in parallel
        all_evaluations = await self.evaluator_registry.evaluate_all(intent, context)
        
        logger.debug(
            "Evaluated with %d evaluators: %s",
            len(all_evaluations),
            [(e.evaluator_name, e.confidence, e.can_handle) for e in all_evaluations],
        )
        
        # Step 3: Select best evaluator
        best_evaluator, used_generation, generation_reason = await self._select_best_evaluator(
            intent,
            all_evaluations,
            intent_result,
        )
        
        # Build result
        if best_evaluator and best_evaluator.can_handle:
            result = SemanticEvaluationResult(
                intent=intent,
                intent_class=intent_result.predicted_class,
                intent_confidence=intent_result.confidence,
                intent_features=list(intent_result.detected_features.keys()),
                evaluator_name=best_evaluator.evaluator_name,
                evaluator_confidence=best_evaluator.confidence,
                can_handle=True,
                reasoning=best_evaluator.reasoning,
                all_evaluations=all_evaluations,
                semantic_features=best_evaluator.semantic_features,
                capability_metadata=best_evaluator.capability_metadata,
                used_generation=used_generation,
                generation_reason=generation_reason,
            )
        else:
            # No evaluator can handle - use LLM to generate novel operation
            result = SemanticEvaluationResult(
                intent=intent,
                intent_class=intent_result.predicted_class,
                intent_confidence=intent_result.confidence,
                intent_features=list(intent_result.detected_features.keys()),
                evaluator_name=None,
                evaluator_confidence=0.0,
                can_handle=False,
                reasoning=generation_reason or "No evaluator matched; will generate novel operation",
                all_evaluations=all_evaluations,
                semantic_features={},
                capability_metadata={},
                used_generation=True,
                generation_reason=generation_reason or "No suitable evaluator found; requires generation",
            )
        
        logger.info(
            "SemanticEngine evaluation complete: evaluator=%s, confidence=%.2f, can_handle=%s",
            result.evaluator_name,
            result.evaluator_confidence,
            result.can_handle,
        )
        
        return result
    
    async def _select_best_evaluator(
        self,
        intent: str,
        evaluations: List[EvaluationResult],
        intent_result: IntentClassificationResult,
    ) -> Tuple[Optional[EvaluationResult], bool, Optional[str]]:
        """
        Select best evaluator from evaluation results.
        
        Strategy:
        1. Filter evaluators that can_handle=True
        2. Sort by confidence (descending)
        3. Take highest confidence if >= min_confidence
        4. Otherwise, trigger generative operation creation (ALWAYS enabled)
        
        Returns:
            Tuple of (best_evaluator, used_generation, generation_reason)
        """
        # Filter evaluators that can handle
        capable = [e for e in evaluations if e.can_handle]
        
        if not capable:
            logger.info("No evaluators can handle intent: '%s'; will generate novel operation", intent)
            # Would invoke LLM here to generate novel function
            # For now, return None to indicate need for generation
            return None, True, "No evaluator matched; will generate novel operation"
        
        # Sort by confidence
        capable.sort(key=lambda e: e.confidence, reverse=True)
        best = capable[0]
        
        # Check minimum confidence
        if best.confidence < self.min_confidence:
            logger.info(
                "Best evaluator confidence %.2f < min %.2f for intent: '%s'; will generate operation",
                best.confidence,
                self.min_confidence,
                intent,
            )
            # Generative operations are ALWAYS enabled
            return None, True, f"Best confidence {best.confidence:.2f} below threshold {self.min_confidence}"
        
        logger.debug(
            "Selected evaluator: %s (confidence=%.2f)",
            best.evaluator_name,
            best.confidence,
        )
        
        return best, False, None
    
    async def find_similar_intents(
        self,
        intent: str,
        candidates: List[str],
        top_k: int = 5,
        threshold: float = 0.7,
    ) -> List[Tuple[str, float]]:
        """
        Find similar intents using semantic embeddings.
        
        Useful for:
        - Suggesting similar operations to user
        - Finding examples for few-shot prompts
        - Detecting duplicate intents
        
        Args:
            intent: Query intent
            candidates: List of candidate intents
            top_k: Maximum results to return
            threshold: Minimum similarity score (0-1)
        
        Returns:
            List of (candidate_intent, similarity_score) tuples
        """
        similar = await self.embeddings.find_most_similar(
            query=intent,
            candidates=candidates,
            top_k=top_k,
            threshold=threshold,
        )
        
        logger.debug(
            "Found %d similar intents for '%s': %s",
            len(similar),
            intent,
            similar,  # Already a list of (text, similarity) tuples
        )
        
        return similar
    
    def get_capabilities(self) -> Dict[str, Any]:
        """
        Get all capabilities of registered evaluators.
        
        Returns:
            Dictionary with evaluator capabilities and statistics
        """
        caps = self.evaluator_registry.get_capabilities()
        
        # Add engine-level metadata
        caps["engine_config"] = {
            "min_confidence": self.min_confidence,
            "generative_operations": "always_enabled",
            "intent_classifier_examples": len(self.intent_classifier.examples),
            "cache_size": len(self.embeddings.cache),
        }
        
        return caps
    
    def add_intent_example(
        self,
        intent: str,
        intent_class: IntentClass,
        features: Optional[List[str]] = None,
    ):
        """
        Add a new few-shot example to intent classifier.
        
        Allows runtime learning from user feedback.
        
        Args:
            intent: Example intent text
            intent_class: Classification for this intent
            features: Optional features detected in intent
        """
        self.intent_classifier.add_example(intent, intent_class, features)
        
        logger.info(
            "Added intent example: class=%s, intent='%s'",
            intent_class.value,
            intent,
        )
    
    async def validate_startup(self) -> Tuple[bool, List[str]]:
        """
        Validate that semantic engine is fully operational.
        
        Per AGENTS.md: "Semantic evaluation is always available core feature."
        Missing semantic infrastructure is a FATAL error.
        
        Returns:
            Tuple of (is_valid, error_messages)
        """
        errors = []
        
        # Check intent classifier
        if not self.intent_classifier:
            errors.append("IntentClassifier not initialized")
        elif len(self.intent_classifier.examples) == 0:
            errors.append("IntentClassifier has no few-shot examples")
        
        # Check evaluator registry
        if not self.evaluator_registry:
            errors.append("EvaluatorRegistry not initialized")
        elif len(self.evaluator_registry.get_all()) == 0:
            errors.append("EvaluatorRegistry has no evaluators registered")
        
        # Check embeddings
        if not self.embeddings:
            errors.append("SemanticEmbeddings not initialized")
        else:
            # Test embedding generation
            try:
                test_result = await self.embeddings.embed("test")
                if not test_result.embedding or len(test_result.embedding) == 0:
                    errors.append("SemanticEmbeddings returned empty embedding")
            except Exception as e:
                errors.append(f"SemanticEmbeddings failed: {str(e)}")
        
        is_valid = len(errors) == 0
        
        if is_valid:
            logger.info("SemanticEngine startup validation: PASSED")
        else:
            logger.error("SemanticEngine startup validation: FAILED - %s", errors)
        
        return is_valid, errors
