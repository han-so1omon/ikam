"""Command contracts for modeling services.

REDESIGNED FOR COMPLETE GENERATIVITY:
- No hardcoded action enums (EconomicAction, StoryAction)
- Commands specify semantic intent and context
- MCP handlers use SemanticEngine to generate operations on-demand
- Every operation is generated, deterministic, and traceable
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Dict, Optional, Callable


@dataclass(frozen=True)
class SemanticEconomicCommand:
    """Economic operation specified semantically (generated on-demand).
    
    No hardcoded actions. User intent is interpreted semantically,
    and the exact function needed is generated to fulfill that intent.
    """
    project_id: str
    instruction: str  # Natural language instruction (e.g., "correlate revenue with market size")
    context: Dict[str, Any]  # Semantic context: extracted parameters, artifact references, etc.
    requested_at: int
    correlation_id: Optional[str] = None
    reply_topic: Optional[str] = None
    
    # Semantic metadata (populated by SemanticEngine)
    semantic_intent: Optional[str] = None  # Classified intent (e.g., "correlate_variables")
    confidence: float = 0.0  # SemanticEngine confidence (0.0-1.0)
    parameters: Optional[Dict[str, Any]] = None  # Extracted parameters for function generation
    generated_function_id: Optional[str] = None  # Reference to generated function (for caching/versioning)


@dataclass(frozen=True)
class SemanticStoryCommand:
    """Story operation specified semantically (generated on-demand).
    
    No hardcoded actions. User intent is interpreted semantically,
    and the exact story operation is generated to fulfill that intent.
    """
    project_id: str
    instruction: str  # Natural language instruction (e.g., "create a narrative arc emphasizing growth")
    context: Dict[str, Any]  # Semantic context
    requested_at: int
    correlation_id: Optional[str] = None
    reply_topic: Optional[str] = None
    
    # Semantic metadata (populated by SemanticEngine)
    semantic_intent: Optional[str] = None  # Classified intent (e.g., "create_narrative_arc")
    confidence: float = 0.0
    parameters: Optional[Dict[str, Any]] = None
    generated_function_id: Optional[str] = None


class EconomicStatus(str, Enum):
    """Outcome status for economic operations."""
    OK = "ok"
    FAILED = "failed"
    PARTIAL = "partial"
    ACCEPTED = "accepted"


@dataclass(frozen=True)
class SemanticEconomicResult:
    """Result of a semantically-generated economic operation."""
    project_id: str
    status: EconomicStatus
    completed_at: int
    payload: Optional[Dict[str, Any]] = None
    error: Optional[str] = None
    correlation_id: Optional[str] = None
    
    # Generative provenance (for IKAM traceability)
    semantic_reasoning: Optional[str] = None  # Why was this operation chosen?
    generated_function_id: Optional[str] = None  # Reference to generated function
    applied_constraints: Optional[Dict[str, Any]] = None  # Which constraints applied?
    generation_metadata: Optional[Dict[str, Any]] = None  # How was the function generated?


class StoryStatus(str, Enum):
    """Outcome status for story operations."""
    OK = "ok"
    FAILED = "failed"
    PARTIAL = "partial"
    ACCEPTED = "accepted"


@dataclass(frozen=True)
class SemanticStoryResult:
    """Result of a semantically-generated story operation."""
    project_id: str
    status: StoryStatus
    completed_at: int
    payload: Optional[Dict[str, Any]] = None
    error: Optional[str] = None
    correlation_id: Optional[str] = None
    
    # Generative provenance
    semantic_reasoning: Optional[str] = None
    generated_function_id: Optional[str] = None
    applied_constraints: Optional[Dict[str, Any]] = None
    generation_metadata: Optional[Dict[str, Any]] = None


# Type aliases for backward compatibility during migration
ModelingCommand = SemanticEconomicCommand | SemanticStoryCommand
ModelingResult = SemanticEconomicResult | SemanticStoryResult


def requires_immediate_reply(command: ModelingCommand) -> bool:
    """Return True if command is expected to respond synchronously.
    
    For generated operations, some are long-running (streaming, generation)
    and some are quick (data retrieval, validation).
    """
    # Streaming/generation operations return ACCEPTED status
    if "stream" in command.instruction.lower():
        return False
    if "generate" in command.instruction.lower() and len(command.instruction) > 50:
        # Long generative instructions may be async
        return False
    
    return True

