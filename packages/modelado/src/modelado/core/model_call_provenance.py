"""Provenance extensions for model calls.

Extends ExecutionProvenanceEvent with model-specific metadata for:
- Tracking model invocations (which model, which parameters, which seed)
- Computing cost and latency per model call
- Recording cache hits/misses
- Enabling deterministic replay via seed
- Supporting Fisher Information calculations for model-generated content

Mathematical foundation:
  I_model_call(θ) includes:
  - θ_model: which model was chosen (inference about model capabilities)
  - θ_params: temperature, top_p, max_tokens (inference about output specificity)
  - θ_seed: deterministic seed (enables exact reproducibility)
  - θ_cost: token costs (inference about prompt complexity)

Usage:
    from modelado.core.model_call_client import ModelCallClient, ModelCallParams, ModelCallResult
    from modelado.core.model_call_provenance import ModelCallProvenanceEvent, ModelCallTracker
    
    # Create client and tracker
    client = ModelCallClient(client_id="gfn_analyst")
    tracker = ModelCallTracker(tracker_id="tracking_123")
    
    # Make a call
    params = ModelCallParams(
        model="gpt-4o-mini",
        prompt="Analyze the elasticity of demand",
        temperature=0.7,
        max_tokens=500,
        seed=42
    )
    result = await client.call(params, use_cache=True)
    
    # Record in provenance
    prov_event = tracker.record_model_call(
        function_id="gfn_abc123",
        model_call_result=result,
        params=params,
        execution_id="exec_xyz789"
    )
"""

from __future__ import annotations

import logging
from datetime import datetime
from typing import Any, Dict, Optional
from uuid import uuid4

from pydantic import BaseModel, ConfigDict, Field

from modelado.core.model_call_client import ModelCallParams, ModelCallResult

logger = logging.getLogger(__name__)


# Model Call Provenance Events

class ModelCallProvenanceEvent(BaseModel):
    """Records model call provenance within function execution.
    
    Extends ExecutionProvenanceEvent context with model-specific metadata.
    
    Fisher Information contribution:
    - θ_model: observing which model was chosen constrains model capability space
    - θ_params: hyperparameters constrain output specificity and determinism
    - θ_seed: deterministic seed enables exact reproduction and debug analysis
    - θ_cost: token usage provides evidence about prompt complexity
    
    Example derivation:
      User intent: "Analyze price elasticity"
      → SemanticEngine generates function (Generation provenance)
      → Function calls GPT-4o-mini with seed=42 (Model Call provenance)
      → Model outputs analysis text (Execution output)
      → I_model_call(θ) = I_generation + I_model + I_cost + I_seed
    """
    
    event_id: str = Field(default_factory=lambda: f"mcall_{uuid4().hex[:16]}")
    timestamp: datetime = Field(default_factory=datetime.utcnow)
    
    # Model identification
    model_name: str = Field(
        ...,
        description="Model used (e.g., 'gpt-4o-mini', 'claude-haiku', 'gpt-4-turbo')"
    )
    model_provider: str = Field(
        ...,
        description="Provider (e.g., 'openai', 'anthropic', 'cohere')"
    )
    
    # Invocation parameters
    prompt_hash: str = Field(
        ...,
        description="BLAKE3 hash of prompt for deduplication"
    )
    seed: Optional[int] = Field(
        None,
        description="Deterministic seed for reproducibility (if set)"
    )
    
    # Hyperparameters (captured for reproducibility and analysis)
    temperature: float = Field(
        ...,
        ge=0.0,
        le=2.0,
        description="Temperature parameter (0=deterministic, higher=more random)"
    )
    max_tokens: int = Field(
        ...,
        ge=1,
        le=4096,
        description="Maximum output tokens"
    )
    top_p: Optional[float] = Field(
        None,
        ge=0.0,
        le=1.0,
        description="Top-p nucleus sampling (if used)"
    )
    
    # Output metadata
    output_hash: str = Field(
        ...,
        description="BLAKE3 hash of model output"
    )
    output_length: int = Field(
        ...,
        ge=0,
        description="Output length in tokens (estimated or actual)"
    )
    
    # Cost tracking (Fisher Information: cost reveals prompt complexity)
    input_tokens: int = Field(
        ...,
        ge=0,
        description="Input tokens (prompt + context)"
    )
    output_tokens: int = Field(
        ...,
        ge=0,
        description="Output tokens (model response)"
    )
    cost_input_usd: float = Field(
        ...,
        ge=0.0,
        description="Cost of input tokens (USD)"
    )
    cost_output_usd: float = Field(
        ...,
        ge=0.0,
        description="Cost of output tokens (USD)"
    )
    total_cost_usd: float = Field(
        ...,
        ge=0.0,
        description="Total cost (USD)"
    )
    
    # Latency tracking (Fisher Information: latency reveals model load/complexity)
    latency_ms: float = Field(
        ...,
        ge=0.0,
        description="Model invocation latency (milliseconds)"
    )
    
    # Cache status
    was_cached: bool = Field(
        ...,
        description="Whether output was retrieved from cache"
    )
    cache_key: Optional[str] = Field(
        None,
        description="Cache key used for lookup (if cached)"
    )
    
    # Context
    function_id: str = Field(
        ...,
        description="Generated function ID that made this call"
    )
    execution_id: str = Field(
        ...,
        description="Execution ID within function"
    )
    artifact_id: Optional[str] = Field(
        None,
        description="Artifact being analyzed (if applicable)"
    )
    user_id: Optional[str] = Field(
        None,
        description="User ID"
    )
    
    # Error tracking
    error: Optional[str] = Field(
        None,
        description="Error message if call failed"
    )
    
    # Reproducibility
    invocation_index: int = Field(
        0,
        ge=0,
        description="Index of this call within function execution (for ordering)"
    )
    
    model_config = ConfigDict(json_schema_extra={
        "example": {
            "event_id": "mcall_abc123def456",
            "timestamp": "2025-12-06T10:30:45.123456Z",
            "model_name": "gpt-4o-mini",
            "model_provider": "openai",
            "prompt_hash": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "seed": 42,
            "temperature": 0.7,
            "max_tokens": 500,
            "output_hash": "d4f7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e4f",
            "output_length": 287,
            "input_tokens": 145,
            "output_tokens": 287,
            "cost_input_usd": 0.000215,
            "cost_output_usd": 0.000862,
            "total_cost_usd": 0.001077,
            "latency_ms": 1230.5,
            "was_cached": False,
            "function_id": "gfn_abc123",
            "execution_id": "exec_xyz789",
            "artifact_id": "art_spreadsheet_123",
            "invocation_index": 0,
        }
    })


class ModelCallBatchProvenanceEvent(BaseModel):
    """Records batch submission of multiple model calls.
    
    Used when batching multiple calls for efficiency.
    
    Fisher Information: Batch grouping reveals parameter patterns
    (similar intents use similar hyperparameters).
    """
    
    event_id: str = Field(default_factory=lambda: f"mbatch_{uuid4().hex[:16]}")
    timestamp: datetime = Field(default_factory=datetime.utcnow)
    
    # Batch identification
    batch_id: str = Field(
        ...,
        description="Unique batch ID"
    )
    model_name: str = Field(
        ...,
        description="Model name"
    )
    
    # Batch statistics
    batch_size: int = Field(
        ...,
        ge=1,
        description="Number of calls in batch"
    )
    param_hash: str = Field(
        ...,
        description="Hash of grouped parameters"
    )
    
    # Cost and latency
    total_cost_usd: float = Field(
        ...,
        ge=0.0,
        description="Total batch cost"
    )
    total_input_tokens: int = Field(
        ...,
        ge=0,
        description="Total input tokens for batch"
    )
    total_output_tokens: int = Field(
        ...,
        ge=0,
        description="Total output tokens for batch"
    )
    
    # Caching impact
    cached_items: int = Field(
        0,
        ge=0,
        description="Number of cached results in batch"
    )
    
    # Context
    submitted_at: datetime = Field(
        default_factory=datetime.utcnow,
        description="When batch was submitted"
    )
    completed_at: Optional[datetime] = Field(
        None,
        description="When batch completed execution"
    )
    
    function_ids: list[str] = Field(
        default_factory=list,
        description="Function IDs that submitted calls in batch"
    )
    
    model_config = ConfigDict(json_schema_extra={
        "example": {
            "event_id": "mbatch_xyz789abc123",
            "batch_id": "batch_123abc",
            "model_name": "gpt-4o-mini",
            "batch_size": 5,
            "param_hash": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "total_cost_usd": 0.0052,
            "total_input_tokens": 725,
            "total_output_tokens": 1435,
            "cached_items": 2,
            "function_ids": ["gfn_abc123", "gfn_def456"],
        }
    })


# Model Call Provenance Tracker

class ModelCallTracker:
    """Records model call provenance events.
    
    Tracks all model calls made during function execution for:
    - Deterministic replay (seed → exact same output)
    - Cost attribution (which functions incur which costs)
    - Cache efficiency (cache hit rates per model)
    - Performance analysis (latency per model and parameter combo)
    
    Usage:
        tracker = ModelCallTracker(tracker_id="tracking_123")
        
        # Record individual call
        prov = tracker.record_model_call(
            function_id="gfn_abc123",
            model_call_result=result,
            params=params,
            execution_id="exec_xyz789"
        )
        
        # Record batch submission
        batch_prov = tracker.record_batch(
            batch_id="batch_123",
            model_name="gpt-4o-mini",
            call_events=[prov1, prov2, prov3]
        )
        
        # Get stats
        stats = tracker.get_stats()
    """
    
    def __init__(self, tracker_id: str):
        """Initialize tracker.
        
        Args:
            tracker_id: Unique tracker ID (often an execution_id)
        """
        self.tracker_id = tracker_id
        self.call_events: list[ModelCallProvenanceEvent] = []
        self.batch_events: list[ModelCallBatchProvenanceEvent] = []
        self._invocation_index = 0
        logger.info(f"ModelCallTracker initialized: {tracker_id}")
    
    def record_model_call(
        self,
        function_id: str,
        model_call_result: ModelCallResult,
        params: ModelCallParams,
        execution_id: str,
        prompt_hash: str = "",
        artifact_id: Optional[str] = None,
        user_id: Optional[str] = None,
    ) -> ModelCallProvenanceEvent:
        """Record a model call in provenance.
        
        Args:
            function_id: Generated function ID
            model_call_result: Result from ModelCallClient.call()
            params: Parameters sent to ModelCallClient
            execution_id: Execution ID
            prompt_hash: Hash of prompt (computed if not provided)
            artifact_id: Artifact ID if applicable
            user_id: User ID
            
        Returns:
            ModelCallProvenanceEvent
        """
        if not prompt_hash:
            # Compute hash from prompt if not provided
            import hashlib
            try:
                prompt_hash = hashlib.blake3(params.prompt.encode()).hexdigest()
            except AttributeError:
                # Fallback to SHA256 if blake3 not available
                prompt_hash = hashlib.sha256(params.prompt.encode()).hexdigest()
        
        # Extract model provider from model name
        model_provider = self._get_provider(params.model)
        
        event = ModelCallProvenanceEvent(
            model_name=params.model,
            model_provider=model_provider,
            prompt_hash=prompt_hash,
            seed=params.seed,
            temperature=params.temperature,
            max_tokens=params.max_tokens,
            top_p=params.top_p,
            output_hash=model_call_result.output_hash,
            output_length=len(model_call_result.output.split()),  # Rough token estimate
            input_tokens=self._estimate_tokens(params.prompt),
            output_tokens=self._estimate_tokens(model_call_result.output),
            cost_input_usd=0.0,  # Will be filled from ModelCallClient cost profile
            cost_output_usd=0.0,
            total_cost_usd=model_call_result.cost_usd,
            latency_ms=model_call_result.latency_ms,
            was_cached=model_call_result.cached,
            cache_key=f"{params.model}_{prompt_hash}_{params.seed}" if params.seed else None,
            function_id=function_id,
            execution_id=execution_id,
            artifact_id=artifact_id,
            user_id=user_id,
            invocation_index=self._invocation_index,
        )
        
        self.call_events.append(event)
        self._invocation_index += 1
        
        logger.debug(
            f"Recorded model call: {event.event_id} | "
            f"{params.model} (seed={params.seed}, cached={model_call_result.cached}, cost=${model_call_result.cost_usd:.4f})"
        )
        
        return event
    
    def record_batch(
        self,
        batch_id: str,
        model_name: str,
        call_events: list[ModelCallProvenanceEvent],
        function_ids: Optional[list[str]] = None,
    ) -> ModelCallBatchProvenanceEvent:
        """Record a batch submission of model calls.
        
        Args:
            batch_id: Batch ID
            model_name: Model name
            call_events: Individual call events in batch
            function_ids: List of function IDs that contributed calls
            
        Returns:
            ModelCallBatchProvenanceEvent
        """
        total_cost = sum(e.total_cost_usd for e in call_events)
        total_input_tokens = sum(e.input_tokens for e in call_events)
        total_output_tokens = sum(e.output_tokens for e in call_events)
        cached_items = sum(1 for e in call_events if e.was_cached)
        
        event = ModelCallBatchProvenanceEvent(
            batch_id=batch_id,
            model_name=model_name,
            batch_size=len(call_events),
            param_hash="",  # Would be filled by queue if available
            total_cost_usd=total_cost,
            total_input_tokens=total_input_tokens,
            total_output_tokens=total_output_tokens,
            cached_items=cached_items,
            function_ids=function_ids or [],
        )
        
        self.batch_events.append(event)
        
        logger.debug(
            f"Recorded batch: {event.event_id} | "
            f"batch_id={batch_id}, size={len(call_events)}, cost=${total_cost:.4f}, cached={cached_items}"
        )
        
        return event
    
    def get_stats(self) -> Dict[str, Any]:
        """Get aggregate statistics about model calls.
        
        Returns:
            Dictionary with stats
        """
        if not self.call_events:
            return {
                "total_calls": 0,
                "total_cost_usd": 0.0,
                "total_input_tokens": 0,
                "total_output_tokens": 0,
                "cache_hits": 0,
                "cache_hit_rate": 0.0,
                "avg_latency_ms": 0.0,
                "models_used": [],
            }
        
        total_cost = sum(e.total_cost_usd for e in self.call_events)
        total_input = sum(e.input_tokens for e in self.call_events)
        total_output = sum(e.output_tokens for e in self.call_events)
        cache_hits = sum(1 for e in self.call_events if e.was_cached)
        avg_latency = sum(e.latency_ms for e in self.call_events) / len(self.call_events)
        models_used = list(set(e.model_name for e in self.call_events))
        
        return {
            "total_calls": len(self.call_events),
            "total_cost_usd": total_cost,
            "total_input_tokens": total_input,
            "total_output_tokens": total_output,
            "cache_hits": cache_hits,
            "cache_hit_rate": cache_hits / len(self.call_events) if self.call_events else 0.0,
            "avg_latency_ms": avg_latency,
            "models_used": models_used,
            "total_batches": len(self.batch_events),
        }
    
    def get_call_events(self) -> list[ModelCallProvenanceEvent]:
        """Get all recorded model call events."""
        return self.call_events
    
    def get_batch_events(self) -> list[ModelCallBatchProvenanceEvent]:
        """Get all recorded batch events."""
        return self.batch_events
    
    # Private helpers
    
    @staticmethod
    def _get_provider(model_name: str) -> str:
        """Extract provider from model name."""
        if "gpt" in model_name.lower():
            return "openai"
        elif "claude" in model_name.lower():
            return "anthropic"
        elif "cohere" in model_name.lower():
            return "cohere"
        else:
            return "unknown"
    
    @staticmethod
    def _estimate_tokens(text: str) -> int:
        """Rough token estimation (4 chars ≈ 1 token, typical for English)."""
        return max(1, len(text) // 4)
