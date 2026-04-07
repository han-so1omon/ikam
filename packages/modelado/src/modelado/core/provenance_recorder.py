"""Provenance recording for generated functions and operations.

Records complete generation and execution provenance for Fisher Information calculation.
Enables:
- Generation provenance (intent, strategy, LLM params, seed, confidence)
- Execution provenance (inputs, outputs, execution path, timing)
- Derivation chains (which artifacts derived from which intents)
- Information dominance validation (I_IKAM ≥ I_baseline + Δ_provenance)

Mathematical foundation: See docs/ikam/FISHER_INFORMATION_GAINS.md

Usage:
    recorder = ProvenanceRecorder()
    
    # Record generation
    gen_event = recorder.record_generation(
        function_id="gfn_abc123",
        metadata=GeneratedFunctionMetadata(...),
    )
    
    # Record execution
    exec_event = recorder.record_execution(
        function_id="gfn_abc123",
        inputs={"revenue": 1000000},
        outputs={"result": 0.85},
        execution_time_ms=45.2,
    )
    
    # Query provenance
    chain = recorder.get_derivation_chain(function_id="gfn_abc123")
"""

from __future__ import annotations

import logging
from dataclasses import dataclass, field
from datetime import datetime
from enum import Enum
from typing import Any, Dict, List, Optional
from uuid import uuid4

from pydantic import BaseModel, Field, ConfigDict

logger = logging.getLogger(__name__)


# Provenance Event Types

class ProvenanceEventType(str, Enum):
    """Types of provenance events."""
    GENERATION = "generation"              # Function/operation generated
    EXECUTION = "execution"                # Function executed
    VALIDATION = "validation"              # Function validated
    DERIVATION = "derivation"              # Artifact derived from another
    CACHE_HIT = "cache_hit"                # Retrieved from cache
    CACHE_MISS = "cache_miss"              # Not found in cache


# Provenance Event Models

class GenerationProvenanceEvent(BaseModel):
    """Records function generation provenance.
    
    Fisher Information contribution: Observing generation metadata constrains
    parameter space for similar intents.
    """
    event_id: str = Field(default_factory=lambda: f"prov_{uuid4().hex[:16]}")
    event_type: ProvenanceEventType = ProvenanceEventType.GENERATION
    timestamp: datetime = Field(default_factory=datetime.utcnow)
    
    # Function identification
    function_id: str = Field(..., description="Generated function ID")
    content_hash: str = Field(..., description="BLAKE3 hash of canonical code")
    
    # Generation metadata (from GeneratedFunctionMetadata)
    user_intent: str = Field(..., description="Original user instruction")
    semantic_intent: str = Field(..., description="Classified semantic intent")
    confidence: float = Field(..., ge=0.0, le=1.0, description="Classification confidence")
    
    # Generation strategy
    strategy: str = Field(..., description="Generation strategy (template/composable/llm)")
    generator_version: str = Field(..., description="Generator version/model")
    llm_params: Optional[Dict[str, Any]] = Field(None, description="LLM generation parameters")
    
    # Semantic reasoning
    semantic_reasoning: Optional[str] = Field(None, description="Why this function was generated")
    extracted_parameters: Optional[Dict[str, Any] | List[str]] = Field(
        None,
        description="Parameters from intent (dict or list of names)",
    )
    constraints_enforced: List[str] = Field(default_factory=list, description="Validation constraints")
    
    # Provenance links
    derived_from: Optional[str] = Field(None, description="Parent function_id if derived")
    cache_key: Optional[str] = Field(None, description="Semantic cache key")
    
    model_config = ConfigDict(
        json_schema_extra={
            "example": {
                "function_id": "gfn_abc123def456",
                "content_hash": "c54149676f1f0bbb1e2e33b72321cdd471bc834ef6623461db158474bc65d49b",
                "user_intent": "Analyze price elasticity with sigmoid curve",
                "semantic_intent": "analyze_elasticity",
                "confidence": 0.92,
                "strategy": "composable_building_blocks",
                "generator_version": "semantic_engine_v2.1.0",
                "llm_params": {"model": "gpt-4o-mini", "temperature": 0.7},
                "semantic_reasoning": "User requested elasticity analysis with sigmoid transform",
                "extracted_parameters": {"variable": "price", "curve_type": "sigmoid"},
            }
        }
    )


class ExecutionProvenanceEvent(BaseModel):
    """Records function execution provenance.
    
    Fisher Information contribution: Observing input/output pairs provides
    evidence about parameter values (domain constraints, data patterns).
    """
    event_id: str = Field(default_factory=lambda: f"prov_{uuid4().hex[:16]}")
    event_type: ProvenanceEventType = ProvenanceEventType.EXECUTION
    timestamp: datetime = Field(default_factory=datetime.utcnow)
    
    # Function identification
    function_id: str = Field(..., description="Executed function ID")
    execution_id: str = Field(default_factory=lambda: f"exec_{uuid4().hex[:16]}")
    
    # Execution inputs/outputs
    inputs: Dict[str, Any] = Field(..., description="Function input parameters")
    outputs: Dict[str, Any] = Field(..., description="Function return values")
    
    # Execution metadata
    execution_time_ms: float = Field(..., ge=0, description="Execution duration (milliseconds)")
    execution_path: Optional[str] = Field(None, description="Execution trace (for debugging)")
    error: Optional[str] = Field(None, description="Error message if execution failed")
    
    # Context
    artifact_id: Optional[str] = Field(None, description="Artifact ID if applied to artifact")
    user_id: Optional[str] = Field(None, description="User who triggered execution")
    
    model_config = ConfigDict(
        json_schema_extra={
            "example": {
                "function_id": "gfn_abc123def456",
                "execution_id": "exec_xyz789abc123",
                "inputs": {"revenue": 1000000, "price": 50.0},
                "outputs": {"elasticity": 0.85, "confidence_interval": [0.75, 0.95]},
                "execution_time_ms": 45.2,
                "artifact_id": "art_spreadsheet_123",
            }
        }
    )


class DerivationProvenanceEvent(BaseModel):
    """Records artifact derivation relationships.
    
    Fisher Information contribution: Derivation chains reveal structural
    dependencies that constrain parameter space across related artifacts.
    """
    event_id: str = Field(default_factory=lambda: f"prov_{uuid4().hex[:16]}")
    event_type: ProvenanceEventType = ProvenanceEventType.DERIVATION
    timestamp: datetime = Field(default_factory=datetime.utcnow)
    
    # Derivation relationship
    source_id: str = Field(..., description="Source artifact/function ID")
    target_id: str = Field(..., description="Derived artifact/function ID")
    derivation_type: str = Field(..., description="Type of derivation (compose, transform, refine)")
    
    # Transformation metadata
    transformation: Optional[str] = Field(None, description="Transformation applied")
    transformation_params: Optional[Dict[str, Any]] = Field(None, description="Transformation parameters")
    
    # Provenance strength
    derivation_strength: float = Field(1.0, ge=0.0, le=1.0, description="How directly derived (0=weak, 1=exact)")

    @property
    def derivation_id(self) -> str:
        """Compatibility alias used by tests."""
        return self.event_id
    
    model_config = ConfigDict(
        json_schema_extra={
            "example": {
                "source_id": "gfn_abc123",
                "target_id": "gfn_def456",
                "derivation_type": "compose",
                "transformation": "add_error_handling",
                "derivation_strength": 0.9,
            }
        }
    )


# Provenance Recorder

@dataclass
class ProvenanceChain:
    """Represents a provenance chain for a function/artifact."""
    root_id: str
    events: List[GenerationProvenanceEvent | ExecutionProvenanceEvent | DerivationProvenanceEvent] = field(default_factory=list)
    generation_count: int = 0
    execution_count: int = 0
    derivation_count: int = 0

    @property
    def generation_event(self) -> Optional[GenerationProvenanceEvent]:
        events = [e for e in self.events if e.event_type == ProvenanceEventType.GENERATION]
        return events[0] if events else None

    @property
    def execution_events(self) -> List[ExecutionProvenanceEvent]:
        return [e for e in self.events if e.event_type == ProvenanceEventType.EXECUTION]

    @property
    def derivation_events(self) -> List[DerivationProvenanceEvent]:
        return [e for e in self.events if e.event_type == ProvenanceEventType.DERIVATION]

    @property
    def statistics(self) -> Dict[str, Any]:
        execution_events = self.execution_events
        if execution_events:
            avg_ms = sum(e.execution_time_ms for e in execution_events) / len(execution_events)
        else:
            avg_ms = 0.0
        return {
            "generation_count": self.generation_count,
            "execution_count": self.execution_count,
            "derivation_count": self.derivation_count,
            "avg_execution_time_ms": avg_ms,
        }
    
    @property
    def total_events(self) -> int:
        """Total provenance events in chain."""
        return len(self.events)
    
    @property
    def has_executions(self) -> bool:
        """Whether chain contains execution events."""
        return self.execution_count > 0
    
    @property
    def fisher_information_sources(self) -> int:
        """Number of Fisher Information sources (generation + execution + derivation)."""
        sources = 0
        if self.generation_count > 0:
            sources += 1
        if self.execution_count > 0:
            sources += 1
        if self.derivation_count > 0:
            sources += 1
        return sources


class ProvenanceRecorder:
    """Records and queries provenance events for generated functions.
    
    Enables Fisher Information calculation by tracking:
    - Generation metadata (θ_generation)
    - Execution history (θ_execution)
    - Derivation chains (θ_structural)
    
    Mathematical guarantee: Each recorded event increases Fisher Information
    I(θ) by Δ_event(θ) ≥ 0 (see FISHER_INFORMATION_GAINS.md).
    """
    
    def __init__(self):
        """Initialize provenance recorder with in-memory storage."""
        self._events: Dict[str, List[GenerationProvenanceEvent | ExecutionProvenanceEvent | DerivationProvenanceEvent]] = {}
        self._chains: Dict[str, ProvenanceChain] = {}
        logger.info("ProvenanceRecorder initialized (in-memory mode)")
    
    def record_generation(
        self,
        function_id: str,
        content_hash: str,
        user_intent: str,
        semantic_intent: str,
        confidence: float,
        strategy: str,
        generator_version: str,
        llm_params: Optional[Dict[str, Any]] = None,
        semantic_reasoning: Optional[str] = None,
        extracted_parameters: Optional[Dict[str, Any] | List[str]] = None,
        constraints_enforced: Optional[List[str]] = None,
        derived_from: Optional[str] = None,
        cache_key: Optional[str] = None,
    ) -> GenerationProvenanceEvent:
        """Record function generation provenance.
        
        Args:
            function_id: Generated function ID
            content_hash: BLAKE3 hash of canonical code
            user_intent: Original user instruction
            semantic_intent: Classified semantic intent
            confidence: Classification confidence (0.0-1.0)
            strategy: Generation strategy (template/composable/llm)
            generator_version: Generator version/model
            llm_params: LLM generation parameters (optional)
            semantic_reasoning: Why this function was generated (optional)
            extracted_parameters: Parameters from intent (optional)
            constraints_enforced: Validation constraints (optional)
            derived_from: Parent function_id if derived (optional)
            cache_key: Semantic cache key (optional)
            
        Returns:
            GenerationProvenanceEvent
        """
        event = GenerationProvenanceEvent(
            function_id=function_id,
            content_hash=content_hash,
            user_intent=user_intent,
            semantic_intent=semantic_intent,
            confidence=confidence,
            strategy=strategy,
            generator_version=generator_version,
            llm_params=llm_params,
            semantic_reasoning=semantic_reasoning,
            extracted_parameters=extracted_parameters,
            constraints_enforced=constraints_enforced or [],
            derived_from=derived_from,
            cache_key=cache_key,
        )
        
        self._record_event(function_id, event)
        
        # Update chain
        if function_id not in self._chains:
            self._chains[function_id] = ProvenanceChain(root_id=function_id)
        self._chains[function_id].generation_count += 1
        
        logger.debug(f"Recorded generation provenance: {event.event_id} for {function_id}")
        return event
    
    def record_execution(
        self,
        function_id: str,
        inputs: Dict[str, Any],
        outputs: Dict[str, Any],
        execution_time_ms: float,
        execution_path: Optional[str] = None,
        error: Optional[str] = None,
        artifact_id: Optional[str] = None,
        user_id: Optional[str] = None,
    ) -> ExecutionProvenanceEvent:
        """Record function execution provenance.
        
        Args:
            function_id: Executed function ID
            inputs: Function input parameters
            outputs: Function return values
            execution_time_ms: Execution duration (milliseconds)
            execution_path: Execution trace (optional, for debugging)
            error: Error message if execution failed (optional)
            artifact_id: Artifact ID if applied to artifact (optional)
            user_id: User who triggered execution (optional)
            
        Returns:
            ExecutionProvenanceEvent
        """
        event = ExecutionProvenanceEvent(
            function_id=function_id,
            inputs=inputs,
            outputs=outputs,
            execution_time_ms=execution_time_ms,
            execution_path=execution_path,
            error=error,
            artifact_id=artifact_id,
            user_id=user_id,
        )
        
        self._record_event(function_id, event)
        
        # Update chain
        if function_id not in self._chains:
            self._chains[function_id] = ProvenanceChain(root_id=function_id)
        self._chains[function_id].execution_count += 1
        
        logger.debug(f"Recorded execution provenance: {event.event_id} for {function_id}")
        return event
    
    def record_derivation(
        self,
        source_id: str,
        target_id: str,
        derivation_type: str,
        transformation: Optional[str] = None,
        transformation_description: Optional[str] = None,
        transformation_params: Optional[Dict[str, Any]] = None,
        derivation_strength: float = 1.0,
    ) -> DerivationProvenanceEvent:
        """Record artifact derivation relationship.
        
        Args:
            source_id: Source artifact/function ID
            target_id: Derived artifact/function ID
            derivation_type: Type of derivation (compose, transform, refine)
            transformation: Transformation applied (optional)
            transformation_params: Transformation parameters (optional)
            derivation_strength: How directly derived (0=weak, 1=exact)
            
        Returns:
            DerivationProvenanceEvent
        """
        event = DerivationProvenanceEvent(
            source_id=source_id,
            target_id=target_id,
            derivation_type=derivation_type,
            transformation=transformation if transformation is not None else transformation_description,
            transformation_params=transformation_params,
            derivation_strength=derivation_strength,
        )
        
        # Record for both source and target
        self._record_event(source_id, event)
        self._record_event(target_id, event)
        
        # Update chains
        for function_id in [source_id, target_id]:
            if function_id not in self._chains:
                self._chains[function_id] = ProvenanceChain(root_id=function_id)
            self._chains[function_id].derivation_count += 1
        
        logger.debug(f"Recorded derivation provenance: {event.event_id} ({source_id} → {target_id})")
        return event
    
    def get_events(self, function_id: str) -> List[GenerationProvenanceEvent | ExecutionProvenanceEvent | DerivationProvenanceEvent]:
        """Get all provenance events for a function.
        
        Args:
            function_id: Function ID to query
            
        Returns:
            List of provenance events (chronological order)
        """
        return self._events.get(function_id, [])
    
    def get_generation_events(self, function_id: str) -> List[GenerationProvenanceEvent]:
        """Get generation provenance events for a function.
        
        Args:
            function_id: Function ID to query
            
        Returns:
            List of generation events
        """
        return [
            event for event in self.get_events(function_id)
            if event.event_type == ProvenanceEventType.GENERATION
        ]
    
    def get_execution_events(self, function_id: str) -> List[ExecutionProvenanceEvent]:
        """Get execution provenance events for a function.
        
        Args:
            function_id: Function ID to query
            
        Returns:
            List of execution events
        """
        return [
            event for event in self.get_events(function_id)
            if event.event_type == ProvenanceEventType.EXECUTION
        ]
    
    def get_derivation_chain(self, function_id: str) -> ProvenanceChain:
        """Get complete provenance chain for a function.
        
        Args:
            function_id: Function ID to query
            
        Returns:
            ProvenanceChain with all events and statistics
        """
        if function_id not in self._chains:
            return ProvenanceChain(root_id=function_id)
        
        chain = self._chains[function_id]
        chain.events = self.get_events(function_id)
        return chain
    
    def has_provenance(self, function_id: str) -> bool:
        """Check if function has any recorded provenance.
        
        Args:
            function_id: Function ID to check
            
        Returns:
            True if provenance exists, False otherwise
        """
        return function_id in self._events and len(self._events[function_id]) > 0
    
    # Private methods
    
    def _record_event(
        self,
        function_id: str,
        event: GenerationProvenanceEvent | ExecutionProvenanceEvent | DerivationProvenanceEvent,
    ) -> None:
        """Record event in storage."""
        if function_id not in self._events:
            self._events[function_id] = []
        self._events[function_id].append(event)
