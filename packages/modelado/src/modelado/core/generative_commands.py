"""Generative operation command contracts for semantic-driven operations.

This module defines command models for semantic-driven operations that replace
hardcoded operation enums. Instead of:

  EconomicAction.RECALCULATE  # Hardcoded, bounded set
  
We now express:

  GenerativeEconomicCommand(
      semantic_intent="adjust_revenue_15_percent",  # User-driven intent
      confidence=0.92,
      extracted_entities={"rate": 0.15}
  )

This enables:
1. Any user intent generates an appropriate operation (no "unsupported" state)
2. SemanticEngine provides intent classification and routing
3. Generated functions execute deterministically with full IKAM provenance
4. Confidence scores guide operation execution and fallback behavior

See docs/planning/GENERATIVE_OPERATIONS_STRATEGY.md for architecture.
"""

from __future__ import annotations

from dataclasses import dataclass, field, asdict
from datetime import datetime
from enum import Enum
from typing import Any, Dict, List, Optional, Literal
from uuid import uuid4
import json


class GenerationStrategy(str, Enum):
    """Strategy used to generate the function."""
    LLM_BASED = "llm_based"  # Full generation via LLM (most flexible)
    COMPOSABLE = "composable_building_blocks"  # Compose from atomic operations
    TEMPLATE = "template_injection"  # Template with semantic slot filling
    UNKNOWN = "unknown"  # Cannot determine strategy


@dataclass
class GenerativeEconomicCommand:
    """Semantic command for economic operations (replaces EconomicAction enum).
    
    Instead of hardcoded EconomicAction enum values, commands express user intent
    semantically. SemanticEngine interprets intent and routes to appropriate handler.
    
    Attributes:
        semantic_intent: User's expressed intent, e.g.:
            - "adjust_revenue_15_percent"
            - "correlate_costs_headcount"
            - "sensitivity_analysis_all_drivers"
            - "what_is_our_wacc"
            - Custom intents never seen before are also valid
        
        confidence: SemanticEngine classification confidence (0.0-1.0).
            > 0.85: High confidence, auto-execute
            0.6-0.85: Medium confidence, ask for confirmation
            < 0.6: Low confidence, ask user to clarify
        
        intent_class: From semantic engine. Always ECONOMIC_FUNCTION.
        
        extracted_entities: Parameters extracted from user instruction.
            Example: {"rate": 0.15, "period": "annual", "apply_to": "revenue"}
        
        context_data: User's artifact/model context (artifact data, project info).
        
        generation_strategy: How function was/will be generated.
        
        source_instruction: Original user instruction (for debugging/audit).
        
        semantic_features: Feature detection results from semantic engine.
            List of features identified in user intent.
        
        parser_confidence: Confidence from instruction parser.
            Combined with semantic confidence for final decision.
    
    Example:
        >>> cmd = GenerativeEconomicCommand(
        ...     semantic_intent="adjust_revenue_growth_rate",
        ...     confidence=0.92,
        ...     extracted_entities={"new_rate": 0.18, "fiscal_year": 2026},
        ...     context_data={"artifact_id": "art-123"},
        ...     source_instruction="Set revenue growth to 18% for 2026"
        ... )
    """
    semantic_intent: str
    confidence: float  # 0.0-1.0 from SemanticEngine
    intent_class: Literal["ECONOMIC_FUNCTION"] = "ECONOMIC_FUNCTION"
    extracted_entities: Dict[str, Any] = field(default_factory=dict)
    context_data: Dict[str, Any] = field(default_factory=dict)
    generation_strategy: GenerationStrategy = GenerationStrategy.LLM_BASED
    source_instruction: Optional[str] = None
    semantic_features: List[str] = field(default_factory=list)
    parser_confidence: Optional[float] = None  # From instruction parser
    
    # Metadata
    command_id: str = field(default_factory=lambda: str(uuid4()))
    created_at: datetime = field(default_factory=datetime.utcnow)
    
    def to_dict(self) -> Dict[str, Any]:
        """Serialize to dictionary."""
        d = asdict(self)
        d['created_at'] = self.created_at.isoformat()
        d['generation_strategy'] = self.generation_strategy.value
        d['intent_class'] = self.intent_class
        return d
    
    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> GenerativeEconomicCommand:
        """Deserialize from dictionary."""
        data = data.copy()
        if 'created_at' in data and isinstance(data['created_at'], str):
            data['created_at'] = datetime.fromisoformat(data['created_at'])
        if 'generation_strategy' in data and isinstance(data['generation_strategy'], str):
            data['generation_strategy'] = GenerationStrategy(data['generation_strategy'])
        return cls(**data)
    
    def combined_confidence(self) -> float:
        """Combined confidence from semantic engine + parser.
        
        If parser confidence available, blend with semantic confidence.
        Otherwise, return semantic confidence only.
        
        Formula: 0.6 * semantic + 0.4 * parser (if parser available)
        """
        if self.parser_confidence is not None:
            return 0.6 * self.confidence + 0.4 * self.parser_confidence
        return self.confidence


@dataclass
class GenerativeStoryCommand:
    """Semantic command for story operations (replaces StoryAction enum).
    
    Same principles as GenerativeEconomicCommand but for narrative/slide operations.
    
    Attributes:
        semantic_intent: User's narrative intent, e.g.:
            - "create_pitch_deck_emphasizing_growth"
            - "add_investor_perspective_to_slides"
            - "apply_blue_orange_theme"
            - "three_act_narrative_unit_economics"
            - Custom narrative structures never expressed before
        
        confidence: SemanticEngine classification confidence (0.0-1.0).
        
        intent_class: Always STORY_OPERATION.
        
        extracted_entities: Narrative parameters.
            Example: {"tone": "investor", "theme_colors": ["blue", "orange"]}
        
        context_data: Project context, audience, existing slides.
        
        generation_strategy: How story structure was/will be generated.
        
        source_instruction: Original user instruction.
        
        semantic_features: Feature detection results from semantic engine.
        
        parser_confidence: Confidence from instruction parser.
    
    Example:
        >>> cmd = GenerativeStoryCommand(
        ...     semantic_intent="create_investor_pitch_growth_focused",
        ...     confidence=0.88,
        ...     extracted_entities={"audience": "investor", "focus": "growth"},
        ...     context_data={"artifact_id": "art-456", "num_slides": 15},
        ...     source_instruction="Create a pitch deck that emphasizes our growth trajectory"
        ... )
    """
    semantic_intent: str
    confidence: float  # 0.0-1.0 from SemanticEngine
    intent_class: Literal["STORY_OPERATION"] = "STORY_OPERATION"
    extracted_entities: Dict[str, Any] = field(default_factory=dict)
    context_data: Dict[str, Any] = field(default_factory=dict)
    generation_strategy: GenerationStrategy = GenerationStrategy.LLM_BASED
    source_instruction: Optional[str] = None
    semantic_features: List[str] = field(default_factory=list)
    parser_confidence: Optional[float] = None  # From instruction parser
    
    # Metadata
    command_id: str = field(default_factory=lambda: str(uuid4()))
    created_at: datetime = field(default_factory=datetime.utcnow)
    
    def to_dict(self) -> Dict[str, Any]:
        """Serialize to dictionary."""
        d = asdict(self)
        d['created_at'] = self.created_at.isoformat()
        d['generation_strategy'] = self.generation_strategy.value
        d['intent_class'] = self.intent_class
        return d
    
    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> GenerativeStoryCommand:
        """Deserialize from dictionary."""
        data = data.copy()
        if 'created_at' in data and isinstance(data['created_at'], str):
            data['created_at'] = datetime.fromisoformat(data['created_at'])
        if 'generation_strategy' in data and isinstance(data['generation_strategy'], str):
            data['generation_strategy'] = GenerationStrategy(data['generation_strategy'])
        return cls(**data)
    
    def combined_confidence(self) -> float:
        """Combined confidence from semantic engine + parser.
        
        If parser confidence available, blend with semantic confidence.
        Otherwise, return semantic confidence only.
        
        Formula: 0.6 * semantic + 0.4 * parser (if parser available)
        """
        if self.parser_confidence is not None:
            return 0.6 * self.confidence + 0.4 * self.parser_confidence
        return self.confidence


@dataclass
class GeneratedOperationResult:
    """Result of executing a generated operation.
    
    Records full execution trace including generation metadata and IKAM provenance.
    
    Attributes:
        command_id: Reference to original GenerativeEconomicCommand or GenerativeStoryCommand.
        
        semantic_intent: Original user intent.
        
        generated_function_id: Stable ID for caching. Hash of (intent, parameters).
        
        function_signature: Parameter schema and return type (for documentation).
        
        execution_result: Actual operation output (model update, slides, etc.).
        
        generation_metadata: LLM tokens, temperature, model version, seed.
            Used for reproducibility and cost tracking.
        
        execution_latency_ms: How long execution took.
        
        confidence_scores: {
            "semantic": 0.92,  # From SemanticEngine
            "parser": 0.89,     # From InstructionParser
            "combined": 0.90,   # Blended score
            "execution_confidence": 0.95  # Did execution succeed?
        }
        
        ikam_provenance: {
            "artifact_id": "art-123",
            "derivation_type": "generative_operation",
            "generation_strategy": "llm_based",
            "semantic_intent": "adjust_revenue_growth",
            "seed": "abc123",  # For reproducibility
            "lossless_reconstruction": True
        }
        
        error: Optional error message if execution failed.
        
        execution_status: "success" | "failed" | "partial"
    
    Example:
        >>> result = GeneratedOperationResult(
        ...     command_id="cmd-123",
        ...     semantic_intent="adjust_revenue_growth",
        ...     generated_function_id="func-abc123",
        ...     execution_result={"new_revenue": 1500000},
        ...     confidence_scores={"combined": 0.92},
        ...     execution_status="success"
        ... )
    """
    command_id: str
    semantic_intent: str
    generated_function_id: str
    execution_result: Dict[str, Any] = field(default_factory=dict)
    generation_metadata: Dict[str, Any] = field(default_factory=dict)
    confidence_scores: Dict[str, float] = field(default_factory=dict)
    ikam_provenance: Dict[str, Any] = field(default_factory=dict)
    execution_latency_ms: float = 0.0
    execution_status: Literal["success", "failed", "partial"] = "success"
    error: Optional[str] = None
    function_signature: Optional[str] = None
    
    # Metadata
    result_id: str = field(default_factory=lambda: str(uuid4()))
    created_at: datetime = field(default_factory=datetime.utcnow)
    
    def to_dict(self) -> Dict[str, Any]:
        """Serialize to dictionary."""
        d = asdict(self)
        d['created_at'] = self.created_at.isoformat()
        return d
    
    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> GeneratedOperationResult:
        """Deserialize from dictionary."""
        data = data.copy()
        if 'created_at' in data and isinstance(data['created_at'], str):
            data['created_at'] = datetime.fromisoformat(data['created_at'])
        return cls(**data)


__all__ = [
    "GenerationStrategy",
    "GenerativeEconomicCommand",
    "GenerativeStoryCommand",
    "GeneratedOperationResult",
]
