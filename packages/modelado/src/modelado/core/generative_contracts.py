"""Generative operation contracts for complete generativity.

Foundation for the "completely generative" operations system.

This module defines the core data contracts that all generative operations follow:
- GenerativeCommand: User intent to be transformed into executable function
- GeneratedOperation: Result of processing GenerativeCommand
- ExecutableFunction: Generated function ready for execution

Key principles:
1. Complete Generativity: All operations generated on-demand, no hardcoded enums
2. Semantic-First: Classification via SemanticEngine, not boolean logic
3. Deterministic: Same input (intent + context + model version) → same output
4. Provenance-Complete: Full audit trail from intent → function → execution → artifacts

Mathematical Guarantees:
- Determinism: hash(command) → same generated_function (cached by content hash)
- Reproducibility: semantic_engine_version + model_version + seed → same output
- Lossless: All generation metadata recorded for reconstruction
"""

from __future__ import annotations

from dataclasses import dataclass, field, asdict
from datetime import datetime
from enum import Enum
from typing import Any, Callable, Dict, List, Optional, Union
from uuid import uuid4
import hashlib
import json


# ============================================================================
# Enums for Validation and Constraint Specification
# ============================================================================

class GenerationStrategy(str, Enum):
    """Strategy used to generate the function."""
    LLM_BASED = "llm_based"  # Generated using LLM (most flexible)
    COMPOSABLE_BUILDING_BLOCKS = "composable_building_blocks"  # Composed from atomic operations (deterministic)
    TEMPLATE_INJECTION = "template_injection"  # Template with semantic slot filling (balanced)
    UNKNOWN = "unknown"  # Used if strategy cannot be determined


class ConstraintType(str, Enum):
    """Types of constraints that can be enforced on generated functions."""
    DETERMINISTIC = "deterministic"  # Function must be deterministic (same input → same output)
    BOUNDS_CHECK = "bounds_check"  # Function must validate input bounds
    OUTPUT_RANGE_VALIDATION = "output_range_validation"  # Function must validate output is in expected range
    CONSISTENCY = "consistency"  # Function output must be consistent with historical outputs
    LOSSLESS_RECONSTRUCTION = "lossless_reconstruction"  # Decomposed artifacts must reconstruct exactly
    TYPE_SAFETY = "type_safety"  # Function must enforce type contracts
    PERFORMANCE = "performance"  # Function must execute within latency bounds


class ValidationStatus(str, Enum):
    """Status of constraint or function validation."""
    PASSED = "passed"
    FAILED = "failed"
    WARNING = "warning"
    NOT_APPLICABLE = "not_applicable"


# ============================================================================
# Core Contracts
# ============================================================================

@dataclass
class ValidationResult:
    """Result of validating a constraint or function."""
    constraint_type: ConstraintType
    status: ValidationStatus
    message: str
    details: Optional[Dict[str, Any]] = None
    timestamp: datetime = field(default_factory=datetime.utcnow)

    def to_dict(self) -> Dict[str, Any]:
        """Serialize to dictionary."""
        d = asdict(self)
        d['constraint_type'] = self.constraint_type.value
        d['status'] = self.status.value
        d['timestamp'] = self.timestamp.isoformat()
        return d

    def is_success(self) -> bool:
        """Check if validation passed."""
        return self.status == ValidationStatus.PASSED


@dataclass
class ValidationResults:
    """Collection of validation results for a function or command."""
    results: List[ValidationResult] = field(default_factory=list)
    passed_count: int = 0
    failed_count: int = 0
    warning_count: int = 0
    overall_status: ValidationStatus = ValidationStatus.PASSED
    checked_at: datetime = field(default_factory=datetime.utcnow)

    def add_result(self, result: ValidationResult) -> None:
        """Add a validation result."""
        self.results.append(result)
        if result.status == ValidationStatus.PASSED:
            self.passed_count += 1
        elif result.status == ValidationStatus.FAILED:
            self.failed_count += 1
            self.overall_status = ValidationStatus.FAILED
        elif result.status == ValidationStatus.WARNING:
            self.warning_count += 1
            if self.overall_status == ValidationStatus.PASSED:
                self.overall_status = ValidationStatus.WARNING

    def is_valid(self) -> bool:
        """Check if all validations passed."""
        return self.overall_status != ValidationStatus.FAILED

    def to_dict(self) -> Dict[str, Any]:
        """Serialize to dictionary."""
        return {
            'results': [r.to_dict() for r in self.results],
            'passed_count': self.passed_count,
            'failed_count': self.failed_count,
            'warning_count': self.warning_count,
            'overall_status': self.overall_status.value,
            'checked_at': self.checked_at.isoformat(),
        }


@dataclass
class ExecutableFunction:
    """Generated function ready for execution.
    
    This represents the actual code/operation that will be executed.
    Contains the function itself plus metadata about how it was generated.
    """
    name: str  # Function name (e.g., "correlate_revenue_market_size")
    language: str  # Programming language ("python", "sql", "typescript")
    code: str  # The actual function code
    # Defaults are provided for back-compat with earlier tests.
    signature: Any = field(default_factory=dict)  # Input/output type signature
    constraints_enforced: List[ConstraintType] = field(default_factory=list)  # Which constraints this function enforces
    generation_strategy: GenerationStrategy = GenerationStrategy.UNKNOWN  # How was this function generated?
    strategy_metadata: Dict[str, Any] = field(default_factory=dict)  # Strategy-specific metadata
    
    # Content hash for deduplication
    function_id: str = field(default="")  # blake3(code) - populated on initialization
    
    # Metadata for reproducibility and caching
    generated_at: datetime = field(default_factory=datetime.utcnow)
    semantic_engine_version: str = "unknown"
    model_version: Optional[str] = None  # For LLM-based: "gpt-4o-mini 2025-11"
    seed: Optional[int] = None  # For determinism
    
    def __post_init__(self):
        """Compute function_id if not already set."""
        if not self.function_id:
            # Use blake3 hash of code for content addressing
            # For now, use sha256 as fallback (blake3 requires external library)
            self.function_id = hashlib.sha256(self.code.encode()).hexdigest()[:16]

    def to_dict(self) -> Dict[str, Any]:
        """Serialize to dictionary."""
        return {
            'name': self.name,
            'language': self.language,
            'code': self.code,
            'signature': self.signature,
            'constraints_enforced': [c.value for c in self.constraints_enforced],
            'generation_strategy': self.generation_strategy.value,
            'strategy_metadata': self.strategy_metadata,
            'function_id': self.function_id,
            'generated_at': self.generated_at.isoformat(),
            'semantic_engine_version': self.semantic_engine_version,
            'model_version': self.model_version,
            'seed': self.seed,
        }


@dataclass
class GenerativeCommand:
    """User intent to be transformed into an executable function.
    
    This is the input contract: captures what the user wants in natural language
    plus context needed to generate the appropriate function.
    
    Key principle: User provides intent, system generates appropriate function.
    """
    command_id: str  # UUID for this command
    user_instruction: str  # Natural language intent (e.g., "Correlate revenue with market size using sigmoid")
    operation_type: str  # Type of operation ("economic_function", "story_operation", "system_function")
    context: Dict[str, Any]  # Project, artifact, persona context needed for generation
    
    # Optional specifications
    parameters: Dict[str, Any] = field(default_factory=dict)  # Optional parameter overrides
    constraints: List[ConstraintType] = field(default_factory=lambda: [ConstraintType.DETERMINISTIC])  # Required constraints
    
    # Metadata for reproducibility
    semantic_engine_version: str = "unknown"
    generation_strategy: Optional[GenerationStrategy] = None  # If specified by user/system
    model_version: Optional[str] = None  # For LLM-based generation
    seed: Optional[int] = None  # For deterministic reproducibility
    
    # Timing
    requested_at: datetime = field(default_factory=datetime.utcnow)
    correlation_id: Optional[str] = None  # For tracing through system
    
    # Request identification
    user_id: Optional[str] = None
    project_id: Optional[str] = None

    @staticmethod
    def create(
        user_instruction: str,
        operation_type: str,
        context: Optional[Dict[str, Any]] = None,
        constraints: Optional[List[ConstraintType]] = None,
        parameters: Optional[Dict[str, Any]] = None,
        generation_strategy: Optional[GenerationStrategy] = None,
        semantic_engine_version: str = "unknown",
        **kwargs
    ) -> GenerativeCommand:
        """Factory method to create a GenerativeCommand."""
        return GenerativeCommand(
            command_id=str(uuid4()),
            user_instruction=user_instruction,
            operation_type=operation_type,
            context=context or {},
            parameters=parameters or {},
            constraints=constraints or [ConstraintType.DETERMINISTIC],
            generation_strategy=generation_strategy,
            semantic_engine_version=semantic_engine_version,
            **kwargs
        )

    def command_hash(self) -> str:
        """Generate deterministic hash of command for caching.
        
        This is used to check if the same command has been processed before.
        Same input → same hash → same cached result.
        """
        # Hash based on: instruction, operation_type, context, constraints, model_version
        hashable = json.dumps(
            {
                'instruction': self.user_instruction,
                'operation_type': self.operation_type,
                'context': self.context,
                'constraints': [c.value for c in self.constraints],
                'model_version': self.model_version,
                'seed': self.seed,
            },
            sort_keys=True,
            default=str,
        )
        return hashlib.sha256(hashable.encode()).hexdigest()

    def to_dict(self) -> Dict[str, Any]:
        """Serialize to dictionary."""
        return {
            'command_id': self.command_id,
            'user_instruction': self.user_instruction,
            'operation_type': self.operation_type,
            'context': self.context,
            'parameters': self.parameters,
            'constraints': [c.value for c in self.constraints],
            'semantic_engine_version': self.semantic_engine_version,
            'generation_strategy': self.generation_strategy.value if self.generation_strategy else None,
            'model_version': self.model_version,
            'seed': self.seed,
            'requested_at': self.requested_at.isoformat(),
            'correlation_id': self.correlation_id,
            'user_id': self.user_id,
            'project_id': self.project_id,
        }


@dataclass
class GeneratedOperation:
    """Result of processing a GenerativeCommand.
    
    This is the output contract: represents the generated function and all metadata
    about how it was generated, validated, and can be executed.
    """
    operation_id: str  # UUID for this operation
    command_id: str  # Reference to the GenerativeCommand that produced this
    generated_function: ExecutableFunction  # The actual function to execute
    
    # Generation metadata
    generation_metadata: Dict[str, Any] = field(default_factory=dict)  # Semantic reasoning, confidence, etc.
    function_id: str = ""  # Content-addressable hash (populated from generated_function)
    
    # Validation and status
    validation_results: ValidationResults = field(default_factory=ValidationResults)
    is_cached: bool = False  # Was this generated function retrieved from cache?
    
    # Performance metrics
    generation_time_ms: float = 0.0
    semantic_confidence: float = 1.0  # 0.0-1.0: confidence this matches user intent
    
    # Timing
    generated_at: datetime = field(default_factory=datetime.utcnow)
    
    # Context
    semantic_reasoning: Optional[str] = None  # Why this function was selected/generated
    selected_evaluator: Optional[str] = None  # Which semantic evaluator selected this operation

    def __post_init__(self):
        """Initialize derived fields."""
        if not self.function_id:
            self.function_id = self.generated_function.function_id

    @staticmethod
    def create(
        command_id: Optional[str] = None,
        generated_function: Optional[ExecutableFunction] = None,
        generation_metadata: Optional[Dict[str, Any]] = None,
        validation_results: Optional[ValidationResults] = None,
        is_cached: bool = False,
        generation_time_ms: float = 0.0,
        semantic_confidence: float = 1.0,
        semantic_reasoning: Optional[str] = None,
        command: Optional[GenerativeCommand] = None,
        **kwargs
    ) -> GeneratedOperation:
        """Factory method to create a GeneratedOperation."""
        if command is not None:
            command_id = command.command_id
        if command_id is None:
            raise TypeError("GeneratedOperation.create() missing required argument: 'command_id'")
        if generated_function is None:
            raise TypeError("GeneratedOperation.create() missing required argument: 'generated_function'")

        return GeneratedOperation(
            operation_id=str(uuid4()),
            command_id=command_id,
            generated_function=generated_function,
            generation_metadata=generation_metadata or {},
            validation_results=validation_results or ValidationResults(),
            is_cached=is_cached,
            generation_time_ms=generation_time_ms,
            semantic_confidence=semantic_confidence,
            semantic_reasoning=semantic_reasoning,
            **kwargs
        )

    def is_valid(self) -> bool:
        """Check if operation is valid (all validations passed)."""
        return self.validation_results.is_valid()

    def can_execute(self) -> bool:
        """Check if this operation is ready to execute.
        
        Must be valid and have a generated function.
        """
        return self.is_valid() and self.generated_function.code and len(self.generated_function.code) > 0

    def to_dict(self) -> Dict[str, Any]:
        """Serialize to dictionary."""
        return {
            'operation_id': self.operation_id,
            'command_id': self.command_id,
            'generated_function': self.generated_function.to_dict(),
            'generation_metadata': self.generation_metadata,
            'function_id': self.function_id,
            'validation_results': self.validation_results.to_dict(),
            'is_cached': self.is_cached,
            'generation_time_ms': self.generation_time_ms,
            'semantic_confidence': self.semantic_confidence,
            'generated_at': self.generated_at.isoformat(),
            'semantic_reasoning': self.semantic_reasoning,
            'selected_evaluator': self.selected_evaluator,
        }


@dataclass
class ConstraintEnforcement:
    """Record of which constraints were enforced and their results."""
    constraint_type: ConstraintType
    enforced: bool  # Was this constraint enforced?
    validation_result: Optional[ValidationResult] = None
    metadata: Dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> Dict[str, Any]:
        """Serialize to dictionary."""
        return {
            'constraint_type': self.constraint_type.value,
            'enforced': self.enforced,
            'validation_result': self.validation_result.to_dict() if self.validation_result else None,
            'metadata': self.metadata,
        }


# ============================================================================
# Type Aliases for Clarity
# ============================================================================

# Functions used for constraint validation
ConstraintValidatorFn = Callable[[ExecutableFunction, Any], ValidationResult]

# Functions used for semantic feature detection
FeatureDetectorFn = Callable[[str], Dict[str, float]]
