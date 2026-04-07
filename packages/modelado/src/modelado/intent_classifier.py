"""
Intent Classification Module for Generative Operations

This module classifies user intents into operation types (economic_function, story_operation,
system_operation) using LLM-based classification with few-shot examples.

Design Principles:
- LLM-based classification (no hardcoded rules)
- Few-shot examples for accuracy
- Confidence scores for transparency
- Feature detection alongside classification
"""

import logging
import os
from dataclasses import dataclass
from enum import Enum
from typing import List, Dict, Any, Optional

from modelado.oraculo.factory import create_ai_client_from_env
from modelado.oraculo.ai_client import GenerateRequest, AIClient
from modelado.oraculo.providers.openai_client import OpenAIUnifiedAIClient

logger = logging.getLogger(__name__)


class IntentClass(str, Enum):
    """Classification categories for user intents."""
    
    ECONOMIC_FUNCTION = "economic_function"
    STORY_OPERATION = "story_operation"
    SYSTEM_OPERATION = "system_operation"
    UNKNOWN = "unknown"


@dataclass
class IntentClassificationResult:
    """Result of intent classification."""
    
    intent: str
    predicted_class: IntentClass
    confidence: float  # 0.0-1.0
    reasoning: str
    detected_features: Dict[str, Any]  # Semantic features detected
    classification_time_ms: float


class IntentClassifier:
    """
    Classify user intents using LLM with few-shot examples.
    
    Categories:
    - economic_function: Revenue adjustments, sensitivity analysis, scenario planning
    - story_operation: Narrative generation, slide creation, theme application
    - system_operation: Project setup, export, import, configuration
    """
    
    # Few-shot examples for classification
    FEW_SHOT_EXAMPLES = [
        # Economic functions
        {
            "intent": "Correlate revenue with market size using a sigmoid curve",
            "class": "economic_function",
            "features": {"uses_mathematical_model": True, "requires_correlation": True},
        },
        {
            "intent": "Run a sensitivity analysis on all cost drivers",
            "class": "economic_function",
            "features": {"needs_sensitivity_analysis": True, "involves_comparison": True},
        },
        {
            "intent": "Adjust Q4 revenue forecast by 15% increase",
            "class": "economic_function",
            "features": {"requires_time_series": True, "involves_adjustment": True},
        },
        {
            "intent": "Compare three pricing strategies: penetration, premium, and freemium",
            "class": "economic_function",
            "features": {"involves_comparison": True, "needs_scenario_planning": True},
        },
        
        # Story operations
        {
            "intent": "Generate a narrative arc emphasizing unit economics improvement",
            "class": "story_operation",
            "features": {"narrative_generation": True, "theme_application": True},
        },
        {
            "intent": "Create slide deck for Q4 investor update",
            "class": "story_operation",
            "features": {"slide_creation": True, "requires_visualization": True},
        },
        {
            "intent": "Apply minimalist theme to all presentation slides",
            "class": "story_operation",
            "features": {"theme_application": True, "styling_operation": True},
        },
        {
            "intent": "Write executive summary highlighting growth trajectory",
            "class": "story_operation",
            "features": {"narrative_generation": True, "summarization": True},
        },
        
        # System operations
        {
            "intent": "Export financial model to Excel with formulas intact",
            "class": "system_operation",
            "features": {"export_operation": True, "format_conversion": True},
        },
        {
            "intent": "Import historical revenue data from CSV",
            "class": "system_operation",
            "features": {"import_operation": True, "data_ingestion": True},
        },
        {
            "intent": "Create new project for Series A fundraising",
            "class": "system_operation",
            "features": {"project_management": True, "initialization": True},
        },
        {
            "intent": "Configure LLM tier to GPT-4 for this project",
            "class": "system_operation",
            "features": {"configuration": True, "settings_management": True},
        },
    ]
    
    CLASSIFICATION_PROMPT_TEMPLATE = """You are an expert at classifying user intents for a financial modeling and storytelling system.

Classify the following user intent into one of these categories:

1. **economic_function**: Operations on financial models (revenue adjustments, sensitivity analysis, scenario planning, correlations, forecasting)
2. **story_operation**: Narrative and presentation operations (slide creation, narrative generation, theme application, executive summaries)
3. **system_operation**: System-level operations (import/export, project setup, configuration, data management)

Few-shot examples:

{examples}

Now classify this intent:

**User Intent:** {intent}

Respond with JSON in this exact format:
{{
    "predicted_class": "economic_function" | "story_operation" | "system_operation" | "unknown",
    "confidence": 0.0-1.0,
    "reasoning": "Brief explanation of classification decision",
    "detected_features": {{
        "feature_name": true/false,
        ...
    }}
}}

Be conservative with confidence scores. Use "unknown" if intent doesn't clearly fit any category."""
    
    def __init__(
        self,
        openai_api_key: str | None = None,
        model: str = "gpt-4o-mini",
        temperature: float = 0.0,
        confidence_threshold: float = 0.7,
        ai_client: AIClient | None = None,
    ):
        """
        Initialize intent classifier.
        
        Args:
            openai_api_key: OpenAI API key
            model: LLM model to use (default: gpt-4o-mini for cost efficiency)
            temperature: Sampling temperature (default: 0.0 for determinism)
            confidence_threshold: Minimum confidence for non-unknown classification
        """
        if ai_client is not None:
            self.ai_client = ai_client
        elif openai_api_key:
            self.ai_client = OpenAIUnifiedAIClient(
                model=model,
                embed_model=os.getenv("LLM_EMBED_MODEL", "text-embedding-3-large"),
                judge_model=os.getenv("LLM_JUDGE_MODEL", model),
                api_key=openai_api_key,
            )
        else:
            self.ai_client = create_ai_client_from_env()
        self.model = model
        self.temperature = temperature
        self.confidence_threshold = confidence_threshold
        
        logger.info(
            "IntentClassifier initialized: model=%s, temp=%.1f, threshold=%.2f",
            model,
            temperature,
            confidence_threshold,
        )
    
    def _format_examples(self) -> str:
        """Format few-shot examples for prompt."""
        formatted = []
        for ex in self.FEW_SHOT_EXAMPLES:
            formatted.append(
                f"Intent: \"{ex['intent']}\"\n"
                f"Class: {ex['class']}\n"
                f"Features: {ex['features']}\n"
            )
        return "\n".join(formatted)
    
    async def classify(self, intent: str) -> IntentClassificationResult:
        """
        Classify user intent using LLM.
        
        Args:
            intent: User's natural language intent
        
        Returns:
            IntentClassificationResult with predicted class and metadata
        """
        import asyncio
        import json
        
        start_time = asyncio.get_event_loop().time()
        
        # Build prompt with few-shot examples
        prompt = self.CLASSIFICATION_PROMPT_TEMPLATE.format(
            examples=self._format_examples(),
            intent=intent,
        )
        
        try:
            # Call LLM for classification
            response = await self.ai_client.generate(
                GenerateRequest(
                    model=self.model,
                    messages=[
                        {
                            "role": "system",
                            "content": "You are an expert intent classifier. Always respond with valid JSON.",
                        },
                        {"role": "user", "content": prompt},
                    ],
                    temperature=self.temperature,
                    response_format={"type": "json_object"},
                    metadata={"component": "IntentClassifier.classify"},
                )
            )

            # Parse response
            content = response.text
            result_dict = json.loads(content)
            
            predicted_class = IntentClass(result_dict["predicted_class"])
            confidence = float(result_dict["confidence"])
            reasoning = result_dict["reasoning"]
            detected_features = result_dict.get("detected_features", {})
            
            # If confidence below threshold, mark as unknown
            if confidence < self.confidence_threshold and predicted_class != IntentClass.UNKNOWN:
                logger.warning(
                    "Classification confidence %.2f below threshold %.2f, marking as unknown",
                    confidence,
                    self.confidence_threshold,
                )
                predicted_class = IntentClass.UNKNOWN
            
            classification_time_ms = (asyncio.get_event_loop().time() - start_time) * 1000
            
            result = IntentClassificationResult(
                intent=intent,
                predicted_class=predicted_class,
                confidence=confidence,
                reasoning=reasoning,
                detected_features=detected_features,
                classification_time_ms=classification_time_ms,
            )
            
            logger.info(
                "Intent classified: class=%s, confidence=%.2f, time=%.2fms",
                predicted_class.value,
                confidence,
                classification_time_ms,
            )
            
            return result
            
        except Exception as exc:
            logger.error("Intent classification failed: intent=%r, error=%s", intent, exc)
            raise
    
    async def classify_batch(self, intents: List[str]) -> List[IntentClassificationResult]:
        """
        Classify multiple intents in parallel.
        
        Args:
            intents: List of user intents
        
        Returns:
            List of IntentClassificationResults in same order as input
        """
        import asyncio
        
        tasks = [self.classify(intent) for intent in intents]
        return await asyncio.gather(*tasks)
    
    def get_examples_for_class(self, intent_class: IntentClass) -> List[Dict[str, Any]]:
        """
        Get few-shot examples for a specific class.
        
        Args:
            intent_class: Intent class to get examples for
        
        Returns:
            List of example dictionaries
        """
        return [
            ex for ex in self.FEW_SHOT_EXAMPLES 
            if ex["class"] == intent_class.value
        ]
    
    def add_example(
        self,
        intent: str,
        intent_class: IntentClass,
        features: Optional[Dict[str, Any]] = None,
    ) -> None:
        """
        Add a new few-shot example to improve classification.
        
        Args:
            intent: Example user intent
            intent_class: Correct classification
            features: Detected features (optional)
        """
        example = {
            "intent": intent,
            "class": intent_class.value,
            "features": features or {},
        }
        
        self.FEW_SHOT_EXAMPLES.append(example)
        logger.info("Added few-shot example: class=%s, total=%d", intent_class.value, len(self.FEW_SHOT_EXAMPLES))
    
    def get_stats(self) -> Dict[str, Any]:
        """Get classifier statistics."""
        class_counts = {}
        for intent_class in IntentClass:
            class_counts[intent_class.value] = len(self.get_examples_for_class(intent_class))
        
        return {
            "model": self.model,
            "temperature": self.temperature,
            "confidence_threshold": self.confidence_threshold,
            "total_examples": len(self.FEW_SHOT_EXAMPLES),
            "examples_by_class": class_counts,
        }
