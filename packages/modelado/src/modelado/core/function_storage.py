"""Generated function storage with CAS deduplication.

Integrates canonicalization with IKAM's content-addressable storage for:
- Deterministic function deduplication
- Provenance tracking (intent → generation → storage)
- BLAKE3-based content addressing
- Storage monotonicity (Δ(N) ≥ 0 for N generated functions)

Mathematical guarantees:
1. CAS property: hash(canonicalize(f1)) = hash(canonicalize(f2)) ⟺ f1 ≡ f2
2. Idempotent storage: store(f) returns same ID regardless of call count
3. Storage monotonicity: S_deduplicated(N) ≤ S_raw(N)
4. Provenance completeness: All generation metadata preserved
"""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from typing import Any, Dict, List, Optional
from uuid import uuid4

from pydantic import BaseModel, Field, ConfigDict


class GeneratedFunctionMetadata(BaseModel):
    """Metadata for a generated function.
    
    Records generation provenance for Fisher Information calculation.
    """
    
    # Function identity (optional caller-provided label)
    #
    # In PostgreSQL CAS mode, the authoritative function identity is content-addressed.
    # In in-memory test mode, we allow callers/tests to reference a stable function_id.
    function_id: Optional[str] = Field(
        None,
        description="Optional caller-provided function identifier (e.g., gfn_mrr_001)",
    )

    # Generation source
    user_intent: str = Field(..., description="Original user instruction")
    semantic_intent: str = Field(..., description="Classified semantic intent")
    confidence: float = Field(..., ge=0.0, le=1.0, description="Intent classification confidence")
    
    # Generation strategy
    strategy: str = Field(..., description="Generation strategy used")
    generator_version: str = Field(..., description="Generator version/model")
    generation_timestamp: datetime = Field(default_factory=datetime.utcnow)
    
    # Semantic reasoning
    semantic_reasoning: Optional[str] = Field(None, description="Why this function was generated")
    extracted_parameters: Optional[Dict[str, Any] | List[str]] = Field(
        None,
        description="Parameters inferred from intent (dict or list of names)",
    )
    constraints_enforced: List[str] = Field(default_factory=list, description="Validation constraints applied")
    
    # Execution context
    context: Optional[Dict[str, Any]] = Field(None, description="Additional context")
    parameters: Dict[str, Any] = Field(default_factory=dict, description="Semantic parameters used for cache keys")
    
    model_config = ConfigDict(
        extra="allow",
        json_schema_extra={
            "example": {
                "user_intent": "Correlate revenue with market size using sigmoid curve",
                "semantic_intent": "correlate_variables",
                "confidence": 0.92,
                "strategy": "composable_building_blocks",
                "generator_version": "semantic_engine_v2.1.0",
                "semantic_reasoning": "User requested sigmoid correlation for revenue vs market size",
                "extracted_parameters": {
                    "var1": "revenue",
                    "var2": "market_size",
                    "curve_type": "sigmoid"
                },
                "constraints_enforced": [
                    "parameter_bounds_check",
                    "output_range_validation",
                    "determinism_guaranteed"
                ]
            }
        }
    )

    # Compatibility aliases for existing tests and upstream callers
    @property
    def generation_strategy(self) -> str:
        return self.strategy

    @property
    def model_name(self) -> Optional[str]:
        return self.generator_version


@dataclass
class GeneratedFunctionRecord:
    """Canonicalized function with CAS storage metadata.
    
    Represents a generated function stored in CAS with complete provenance.
    """
    
    # CAS identity
    function_id: str  # Content-addressable ID (BLAKE3 of canonical code)
    content_hash: str  # Full BLAKE3 hash for integrity verification
    
    # Function code
    canonical_code: str  # Canonicalized Python code
    original_code: str  # Original generated code (pre-canonicalization)
    
    # Canonicalization provenance
    transformations_applied: List[str] = field(default_factory=list)
    is_semantically_equivalent: bool = True
    
    # Generation metadata
    metadata: GeneratedFunctionMetadata = field(default_factory=lambda: None)
    
    # Storage metadata
    stored_at: Optional[datetime] = None
    storage_key: Optional[str] = None  # CAS storage key (if stored)
    deduplicated: bool = False  # True if this was a duplicate on storage
    original_storage_key: Optional[str] = None  # If deduplicated, points to original
    
    # Execution cache
    cache_key: Optional[str] = None  # Semantic cache key (intent + params hash)
    execution_count: int = 0  # How many times this function has been executed
    
    def to_dict(self) -> Dict[str, Any]:
        """Serialize to dictionary for storage."""
        return {
            "function_id": self.function_id,
            "content_hash": self.content_hash,
            "canonical_code": self.canonical_code,
            "original_code": self.original_code,
            "transformations_applied": self.transformations_applied,
            "is_semantically_equivalent": self.is_semantically_equivalent,
            "metadata": self.metadata.model_dump() if self.metadata else None,
            "stored_at": self.stored_at.isoformat() if self.stored_at else None,
            "storage_key": self.storage_key,
            "deduplicated": self.deduplicated,
            "original_storage_key": self.original_storage_key,
            "cache_key": self.cache_key,
            "execution_count": self.execution_count,
        }
    
    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> GeneratedFunctionRecord:
        """Deserialize from dictionary."""
        metadata = None
        if data.get("metadata"):
            metadata = GeneratedFunctionMetadata(**data["metadata"])
        
        stored_at = None
        if data.get("stored_at"):
            stored_at = datetime.fromisoformat(data["stored_at"])
        
        return cls(
            function_id=data["function_id"],
            content_hash=data["content_hash"],
            canonical_code=data["canonical_code"],
            original_code=data["original_code"],
            transformations_applied=data.get("transformations_applied", []),
            is_semantically_equivalent=data.get("is_semantically_equivalent", True),
            metadata=metadata,
            stored_at=stored_at,
            storage_key=data.get("storage_key"),
            deduplicated=data.get("deduplicated", False),
            original_storage_key=data.get("original_storage_key"),
            cache_key=data.get("cache_key"),
            execution_count=data.get("execution_count", 0),
        )


class FunctionStorageStats(BaseModel):
    """Storage statistics for function deduplication analysis."""
    
    total_functions_generated: int = Field(..., description="Total functions generated")
    unique_functions_stored: int = Field(..., description="Unique functions in CAS")
    duplicate_count: int = Field(..., description="Duplicates detected")
    storage_savings_bytes: int = Field(..., description="Bytes saved via deduplication")
    storage_savings_percent: float = Field(..., description="Percentage saved")
    
    # Storage monotonicity validation
    raw_storage_bytes: int = Field(..., description="Bytes if all functions stored separately")
    deduplicated_storage_bytes: int = Field(..., description="Actual bytes stored in CAS")
    monotonicity_delta: int = Field(..., description="Δ(N) = raw - dedup (should be ≥ 0)")
    
    model_config = ConfigDict(
        json_schema_extra={
            "example": {
                "total_functions_generated": 100,
                "unique_functions_stored": 45,
                "duplicate_count": 55,
                "storage_savings_bytes": 125_000,
                "storage_savings_percent": 55.0,
                "raw_storage_bytes": 227_000,
                "deduplicated_storage_bytes": 102_000,
                "monotonicity_delta": 125_000
            }
        }
    )
