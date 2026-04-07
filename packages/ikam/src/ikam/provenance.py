"""Provenance tracking for IKAM v2 fragments.

This module implements provenance storage and Fisher Information instrumentation
per Task 5 of the IKAM v2 MVP roadmap. Provenance tracking enables:

1. **Derivation chains**: Track "fragment B derived from fragment A via operation O"
2. **Fisher Information gains**: Record relationships that add information about generative parameters
3. **Mathematical guarantees**: I_IKAM(θ) ≥ I_RAG(θ) + Δ_provenance_FI(θ)

Mathematical Framework:
    Let θ denote generative parameters (style, intent, structure).
    Let A be artifact content, Y be provenance (relationships, hierarchy).
    
    Fisher Information Chain Rule:
        I((A,Y); θ) = I(A; θ) + E[I(Y; θ | A)]
        
    Where:
        I(A; θ) = RAG baseline (flat content)
        E[I(Y; θ | A)] = Δ_provenance_FI(θ) ≥ 0 (provenance increment)
        
    IKAM Advantage:
        I_IKAM(θ) = I_RAG(θ) + Δ_provenance_FI(θ) ≥ I_RAG(θ)

Provenance Types:
    - DECOMPOSITION: Fragment extracted from artifact (artifact → fragment)
    - REUSE: Fragment shared across artifacts (fragment → multiple uses)
    - DELTA: Mutation/edit from base fragment (base_fragment → variant_fragment)
    - VARIATION: Non-deterministic render variant (canonical → variant)
    - STRUCTURAL: Hierarchical relationship (parent_fragment → child_fragment)

Storage Note:
    Provenance/derivation relationships are tracked as append-only edge events
    and projected into graph backends for traversal. See:
    - docs/ikam/GRAPH_EDGE_EVENT_LOG.md
    - docs/ikam/HUGEGRAPH_SCHEMA_AND_LOADER_FORMAT.md

References:
    - docs/ikam/FISHER_INFORMATION_GAINS.md (mathematical framework)
    - docs/ikam/MUTATION_AND_VARIATION_MODEL.md (delta and variation semantics)
    - packages/ikam/src/ikam/almacen/postgres.py (storage backend)
"""
from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from enum import Enum
from typing import Any, Dict, Optional


@dataclass
class ProvenanceChain:
    """A complete provenance chain from sources to a target artifact.

    This is a *domain* type: it does not imply any particular storage engine.
    Traversal/backends (e.g., relational Postgres or HugeGraph) materialize
    these chains from their respective projections.
    """

    target_artifact_id: str
    chain_length: int
    artifacts: list[str]  # Ordered from oldest source to target
    derivations: list[str]  # Ordered derivation IDs
    derivation_types: list[str]  # Ordered derivation types


class DerivationType(str, Enum):
    """Type of derivation relationship between fragments.
    
    Each type contributes differently to Fisher Information:
    - DECOMPOSITION: Adds I_structure (hierarchical constraints)
    - REUSE: Adds I_reuse ≈ (M-1)·I_consistency (cross-artifact constraints)
    - DELTA: Adds I_delta (mutation semantics) if delta correlates with θ
    - VARIATION: Adds I_variation if variation policy depends on θ
    - STRUCTURAL: Adds I_hierarchy (parent-child semantic consistency)
    """
    
    DECOMPOSITION = "decomposition"  # Artifact → fragments (forja.decompose)
    REUSE = "reuse"                  # Fragment used in multiple artifacts
    DELTA = "delta"                  # Base fragment → variant (mutation tracking)
    VARIATION = "variation"          # Canonical → render variant (non-deterministic)
    STRUCTURAL = "structural"        # Parent fragment → child fragment (hierarchy)
    MODEL_CALL = "model_call"        # LLM invocation producing derived content
    INVOCATION = "invocation"        # Fragment/function invokes another fragment/function
    TRAVERSAL = "traversal"          # Threaded traversal steps/merges recorded as edges


@dataclass
class DerivationRecord:
    """Record of a derivation relationship between fragments.
    
    Tracks provenance edges in the fragment graph for Fisher Information
    calculation and derivation chain reconstruction.
    
    Attributes:
        source_key: Source fragment identifier (blake3:hash)
        target_key: Target fragment identifier (blake3:hash)
        derivation_type: Type of derivation (decomposition, reuse, delta, etc.)
        operation: Operation name (e.g., "decompose_document", "apply_delta")
        metadata: Operation-specific metadata (e.g., delta_size, seed, renderer_version)
        fisher_info_contribution: Estimated Δ_provenance_FI for this edge (bits)
        created_at: Timestamp when derivation was recorded
        
    Mathematical Interpretation:
        Each edge contributes to total Fisher Information:
            I_IKAM(θ) = I_RAG(θ) + Σ(fisher_info_contribution)
            
        Where fisher_info_contribution ≥ 0 for each edge (non-negativity guarantee).
        
    Example:
        # Fragment reuse across 3 artifacts
        record = DerivationRecord(
            source_key="blake3:abc123...",
            target_key="artifact:output_1",
            derivation_type=DerivationType.REUSE,
            operation="embed_fragment",
            metadata={"artifact_count": 3, "salience": 0.9},
            fisher_info_contribution=2.5  # (3-1) * I_consistency ≈ 2.5 bits
        )
    """
    
    source_key: str
    target_key: str
    derivation_type: DerivationType
    operation: Optional[str] = None
    metadata: Dict[str, Any] = field(default_factory=dict)
    fisher_info_contribution: Optional[float] = None  # Δ_provenance_FI in bits
    created_at: datetime = field(default_factory=datetime.now)
    
    def to_dict(self) -> Dict[str, Any]:
        """Convert to dictionary for storage."""
        return {
            "source_key": self.source_key,
            "target_key": self.target_key,
            "derivation_type": self.derivation_type.value,
            "operation": self.operation,
            "metadata": self.metadata,
            "fisher_info_contribution": self.fisher_info_contribution,
            "created_at": self.created_at.isoformat(),
        }
    
    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> DerivationRecord:
        """Restore from dictionary."""
        return cls(
            source_key=data["source_key"],
            target_key=data["target_key"],
            derivation_type=DerivationType(data["derivation_type"]),
            operation=data.get("operation"),
            metadata=data.get("metadata", {}),
            fisher_info_contribution=data.get("fisher_info_contribution"),
            created_at=datetime.fromisoformat(data["created_at"])
                if isinstance(data.get("created_at"), str)
                else data.get("created_at", datetime.now()),
        )


@dataclass
class ModelCallProvenance:
    """Metadata for a model call edge (Phase 9.7).

    Recorded in DerivationRecord.metadata when derivation_type=MODEL_CALL.
    Keeps deterministic replay details for FI calculations.
    """

    model: str
    prompt_hash: str
    output_hash: str
    seed: Optional[int]
    cost_usd: float
    tokens_input: int
    tokens_output: int
    latency_ms: float
    cached: bool
    temperature: float

    def to_dict(self) -> Dict[str, Any]:
        return {
            "model": self.model,
            "prompt_hash": self.prompt_hash,
            "output_hash": self.output_hash,
            "seed": self.seed,
            "cost_usd": self.cost_usd,
            "tokens_input": self.tokens_input,
            "tokens_output": self.tokens_output,
            "latency_ms": self.latency_ms,
            "cached": self.cached,
            "temperature": self.temperature,
        }

    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> "ModelCallProvenance":
        return cls(
            model=data["model"],
            prompt_hash=data["prompt_hash"],
            output_hash=data["output_hash"],
            seed=data.get("seed"),
            cost_usd=float(data.get("cost_usd", 0.0)),
            tokens_input=int(data.get("tokens_input", 0)),
            tokens_output=int(data.get("tokens_output", 0)),
            latency_ms=float(data.get("latency_ms", 0.0)),
            cached=bool(data.get("cached", False)),
            temperature=float(data.get("temperature", 0.0)),
        )


@dataclass
class ProvenanceMetadata:
    """Metadata for fragment provenance (stored in Fragment.metadata).
    
    This structure extends Fragment records with provenance tracking
    without changing the Fragment model itself (backward compatible).
    
    Attributes:
        derived_from: Source fragment key (if this is a derived fragment)
        derivation_type: Type of derivation relationship
        parent_fragment_id: Parent in hierarchy (redundant with Fragment.parent_fragment_id)
        salience: Fragment salience score (0.0-1.0, redundant with Fragment.salience)
        reuse_count: Number of artifacts using this fragment
        delta_size: Size of delta if derivation_type=DELTA (bytes)
        variation_seed: Seed for variation rendering if derivation_type=VARIATION
        renderer_version: Renderer version for reproducibility
        
    Fisher Information Annotations:
        - reuse_count: I_reuse ≈ (reuse_count - 1) * I_consistency
        - delta_size: Smaller deltas → higher compression → may indicate θ-relevant patterns
        - variation_seed: Enables exact reproduction → preserves FI
    """
    
    derived_from: Optional[str] = None  # Source fragment key
    derivation_type: Optional[DerivationType] = None
    parent_fragment_id: Optional[str] = None  # Hierarchical parent
    salience: float = 0.5  # Default salience
    reuse_count: int = 1  # Number of artifacts using this fragment
    delta_size: Optional[int] = None  # Delta size in bytes (if applicable)
    variation_seed: Optional[int] = None  # Seed for variation rendering
    renderer_version: Optional[str] = None  # Renderer version for reproducibility
    policy_id: Optional[str] = None  # Render policy identifier
    
    def to_dict(self) -> Dict[str, Any]:
        """Convert to dictionary for JSON storage."""
        return {
            k: v.value if isinstance(v, DerivationType) else v
            for k, v in self.__dict__.items()
            if v is not None
        }
    
    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> ProvenanceMetadata:
        """Restore from dictionary."""
        data = data.copy()
        if "derivation_type" in data and data["derivation_type"]:
            data["derivation_type"] = DerivationType(data["derivation_type"])
        return cls(**{k: v for k, v in data.items() if k in cls.__annotations__})
