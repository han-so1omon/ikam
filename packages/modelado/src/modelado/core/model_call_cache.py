"""IKAM cache integration for model outputs.

Wires model call outputs to content-addressable storage (CAS) for:
- Deterministic caching by (model, prompt_hash, seed) tuple
- Lossless reconstruction from fragments
- Fisher Information tracking of cached outputs
- Integration with IKAM artifact versioning

Mathematical foundation:
  Model output caching reduces S_model (storage cost) while maintaining I_model (information)
  Storage monotonicity: Δ_cache(N) = S_flat(N) - S_IKAM_cached(N) ≥ 0
  
Architecture:
    ModelCallResult
    ↓ (store in CAS)
  ikam.graph.StoredFragment (storage layer, immutable)
    ↓ (query for replay)
  ModelCallCacheFragment (domain layer with metadata)
    ↓ (integrate into artifact)
  ikam.fragments.Fragment (IKAM domain model)

Usage:
    from modelado.core.model_call_cache import ModelCallCASCache, ModelCallCacheKey
    from modelado.core.model_call_client import ModelCallClient
    
    # Initialize cache
    cache = ModelCallCASCache(connection_pool=db_pool)
    
    # Make a call
    client = ModelCallClient(client_id="gfn_analyst")
    params = ModelCallParams(model="gpt-4o-mini", prompt="...", seed=42)
    result = await client.call(params)
    
    # Store in CAS
    key = ModelCallCacheKey(
        model=params.model,
        prompt_hash=result.prompt_hash,
        seed=params.seed
    )
    cache_fragment = await cache.store_model_call(
        key=key,
        model_call_result=result,
        artifact_id="art_123"
    )
    
    # Retrieve for deterministic replay
    cached_result = await cache.get_model_call(key)
    
    # List all model calls for artifact
    calls = await cache.get_artifact_model_calls(artifact_id="art_123")
"""

from __future__ import annotations

import logging
from dataclasses import dataclass
from datetime import datetime
from typing import Any, Dict, Optional, TYPE_CHECKING
from uuid import uuid4

import psycopg
from pydantic import BaseModel, Field

from ikam.graph import StoredFragment

if TYPE_CHECKING:  # Import only for type checking to avoid circular dependency
    from modelado.core.invocation_edges import InvocationGraph

logger = logging.getLogger(__name__)


# Model Call Cache Key

@dataclass(frozen=True)
class ModelCallCacheKey:
    """Immutable cache key for model outputs.
    
    Uniquely identifies a model call via (model, prompt_hash, seed).
    Used for CAS deduplication and deterministic replay.
    
    Frozen dataclass allows use as dict key for in-memory caching.
    """
    
    model: str = Field(..., description="Model name (e.g., 'gpt-4o-mini')")
    prompt_hash: str = Field(..., description="BLAKE3 hash of prompt")
    seed: Optional[int] = Field(None, description="Deterministic seed (if used)")
    
    def __hash__(self) -> int:
        """Hash for dict/set use."""
        return hash((self.model, self.prompt_hash, self.seed))
    
    def __eq__(self, other: Any) -> bool:
        """Equality check."""
        if not isinstance(other, ModelCallCacheKey):
            return False
        return (self.model == other.model and 
                self.prompt_hash == other.prompt_hash and 
                self.seed == other.seed)
    
    def to_cache_id(self) -> str:
        """Generate a stable cache ID for fragment association."""
        import hashlib
        key_str = f"{self.model}:{self.prompt_hash}:{self.seed or 'none'}"
        return hashlib.sha256(key_str.encode()).hexdigest()


class ModelCallCacheFragment(BaseModel):
    """Domain-layer wrapper for cached model output.
    
    Associates ModelCallProvenanceEvent metadata with IKAM storage layer.
    
    Enables:
    - Artifact-scoped model call tracking (which model calls touched this artifact)
    - Batch retrieval of all model calls for reproducibility
    - Cost accounting per artifact (sum of model call costs)
    """
    
    # Fragment reference (storage layer)
    fragment_id: str = Field(..., description="BLAKE3 hash of model output (storage layer id)")
    
    # Cache metadata
    model: str = Field(..., description="Model name")
    prompt_hash: str = Field(..., description="Hash of input prompt")
    seed: Optional[int] = Field(None, description="Deterministic seed")
    cache_key_id: str = Field(..., description="Stable cache ID for lookups")
    
    # Artifact association
    artifact_id: Optional[str] = Field(None, description="Artifact ID if applied to artifact")
    function_id: str = Field(..., description="Generated function that created this call")
    execution_id: str = Field(..., description="Execution ID")
    
    # Provenance
    cost_usd: float = Field(..., ge=0.0, description="Model call cost")
    latency_ms: float = Field(..., ge=0.0, description="Model call latency")
    cached_at: datetime = Field(default_factory=datetime.utcnow, description="When stored in cache")
    
    # Content info
    output_hash: str = Field(..., description="Hash of model output for verification")
    output_length: int = Field(..., ge=0, description="Output length in tokens")


# IKAM Model Call Cache

class ModelCallCASCache:
    """Persistent cache for model outputs using IKAM storage.
    
    Implements CAS (content-addressable storage) for model call outputs:
    - Deterministic by (model, prompt_hash, seed)
    - Immutable (append-only, no updates)
    - Integrated with IKAM fragments
    - Supports batch retrieval for reproducibility
    
    Mathematical properties:
    1. Storage Monotonicity: Caching reduces total storage vs flat approach
    2. Information Preservation: Cache hit doesn't lose information
    3. Deterministic Retrieval: Same (model, prompt, seed) → same output
    4. Lossless: Output reconstructable from CAS fragment bytes
    """
    
    def __init__(
        self,
        connection_pool: psycopg.ConnectionPool,
        invocation_graph: Optional["InvocationGraph"] = None,
    ):
        """Initialize cache with database connection pool.
        
        Args:
            connection_pool: psycopg ConnectionPool for database access
            invocation_graph: Optional InvocationGraph for recording function → model call edges
        """
        self.connection_pool = connection_pool
        self.invocation_graph = invocation_graph
        logger.info("ModelCallCASCache initialized")
    
    async def store_model_call(
        self,
        key: ModelCallCacheKey,
        model_call_result: "ModelCallResult",  # noqa: F821
        artifact_id: Optional[str] = None,
        function_id: Optional[str] = None,
        execution_id: Optional[str] = None,
    ) -> ModelCallCacheFragment:
        """Store a model call result in CAS.
        
        Creates a Fragment (storage layer) and returns domain-level metadata.
        Idempotent: calling twice with same result is a cache hit.
        
        Args:
            key: Cache key (model, prompt_hash, seed)
            model_call_result: Result from ModelCallClient.call()
            artifact_id: Artifact this call was made for (optional)
            function_id: Generated function ID
            execution_id: Execution ID
            
        Returns:
            ModelCallCacheFragment with storage reference
        """
        from modelado.core.model_call_client import ModelCallResult
        
        # Create Fragment from model output
        output_bytes = model_call_result.output.encode('utf-8')
        fragment = StoredFragment.from_bytes(output_bytes, mime_type="text/plain")
        
        # Store in database (idempotent on content hash)
        with self.connection_pool.connection() as cx:
            with cx.cursor() as cur:
                # Check for existing fragment (CAS hit)
                cur.execute(
                    "SELECT 1 FROM ikam_fragments WHERE id = %s",
                    (fragment.id,)
                )
                exists = cur.fetchone() is not None
                
                # Insert fragment (ON CONFLICT DO NOTHING for idempotency)
                cur.execute(
                    """
                    INSERT INTO ikam_fragments (id, mime_type, size, bytes)
                    VALUES (%s, %s, %s, %s)
                    ON CONFLICT (id) DO NOTHING
                    """,
                    (fragment.id, fragment.mime_type, fragment.size, fragment.bytes)
                )
                
                # Insert cache metadata
                cache_key_id = key.to_cache_id()
                cur.execute(
                    """
                    INSERT INTO model_call_cache (
                        cache_key_id, model, prompt_hash, seed, fragment_id,
                        artifact_id, function_id, execution_id, cost_usd, latency_ms,
                        output_hash, output_length, cached_at
                    )
                    VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
                    ON CONFLICT (cache_key_id) DO NOTHING
                    """,
                    (
                        cache_key_id,
                        key.model,
                        key.prompt_hash,
                        key.seed,
                        fragment.id,
                        artifact_id,
                        function_id,
                        execution_id,
                        model_call_result.cost_usd,
                        model_call_result.latency_ms,
                        model_call_result.output_hash,
                        len(model_call_result.output.split()),  # Token estimate
                        datetime.utcnow(),
                    )
                )
                
                cx.commit()
        
        logger.debug(
            f"Stored model call in CAS: {key.model} "
            f"(prompt_hash={key.prompt_hash[:8]}..., seed={key.seed}, "
            f"fragment_id={fragment.id[:8]}..., exists={exists})"
        )
        
        cache_fragment = ModelCallCacheFragment(
            fragment_id=fragment.id,
            model=key.model,
            prompt_hash=key.prompt_hash,
            seed=key.seed,
            cache_key_id=key.to_cache_id(),
            artifact_id=artifact_id,
            function_id=function_id or "",
            execution_id=execution_id or "",
            cost_usd=model_call_result.cost_usd,
            latency_ms=model_call_result.latency_ms,
            output_hash=model_call_result.output_hash,
            output_length=len(model_call_result.output.split()),
        )

        if self.invocation_graph and function_id:
            await self.invocation_graph.add_edge(function_id, cache_fragment)

        return cache_fragment
    
    async def get_model_call(
        self,
        key: ModelCallCacheKey,
    ) -> Optional[ModelCallCacheFragment]:
        """Retrieve a cached model call.
        
        Args:
            key: Cache key (model, prompt_hash, seed)
            
        Returns:
            ModelCallCacheFragment if found, None otherwise
        """
        cache_key_id = key.to_cache_id()
        
        with self.connection_pool.connection() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    """
                    SELECT fragment_id, model, prompt_hash, seed, artifact_id,
                           function_id, execution_id, cost_usd, latency_ms,
                           output_hash, output_length, cached_at
                    FROM model_call_cache
                    WHERE cache_key_id = %s
                    """,
                    (cache_key_id,)
                )
                row = cur.fetchone()
        
        if not row:
            return None
        
        return ModelCallCacheFragment(
            fragment_id=row[0],
            model=row[1],
            prompt_hash=row[2],
            seed=row[3],
            cache_key_id=cache_key_id,
            artifact_id=row[4],
            function_id=row[5],
            execution_id=row[6],
            cost_usd=row[7],
            latency_ms=row[8],
            output_hash=row[9],
            output_length=row[10],
            cached_at=row[11],
        )
    
    async def get_artifact_model_calls(
        self,
        artifact_id: str,
    ) -> list[ModelCallCacheFragment]:
        """Get all model calls made for an artifact.
        
        Used for reproducibility and cost accounting.
        
        Args:
            artifact_id: Artifact ID
            
        Returns:
            List of ModelCallCacheFragment (chronological order)
        """
        with self.connection_pool.connection() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    """
                    SELECT fragment_id, model, prompt_hash, seed, artifact_id,
                           function_id, execution_id, cost_usd, latency_ms,
                           output_hash, output_length, cached_at, cache_key_id
                    FROM model_call_cache
                    WHERE artifact_id = %s
                    ORDER BY cached_at ASC
                    """,
                    (artifact_id,)
                )
                rows = cur.fetchall()
        
        return [
            ModelCallCacheFragment(
                fragment_id=row[0],
                model=row[1],
                prompt_hash=row[2],
                seed=row[3],
                cache_key_id=row[12],
                artifact_id=row[4],
                function_id=row[5],
                execution_id=row[6],
                cost_usd=row[7],
                latency_ms=row[8],
                output_hash=row[9],
                output_length=row[10],
                cached_at=row[11],
            )
            for row in rows
        ]
    
    async def get_cache_stats(self) -> Dict[str, Any]:
        """Get aggregate cache statistics.
        
        Returns:
            Dictionary with stats
        """
        with self.connection_pool.connection() as cx:
            with cx.cursor() as cur:
                # Total cached calls
                cur.execute("SELECT COUNT(*) FROM model_call_cache")
                total_calls = cur.fetchone()[0]
                
                # Total cost
                cur.execute("SELECT COALESCE(SUM(cost_usd), 0.0) FROM model_call_cache")
                total_cost = cur.fetchone()[0]
                
                # Average latency
                cur.execute("SELECT COALESCE(AVG(latency_ms), 0.0) FROM model_call_cache")
                avg_latency = cur.fetchone()[0]
                
                # Unique models
                cur.execute("SELECT COUNT(DISTINCT model) FROM model_call_cache")
                unique_models = cur.fetchone()[0]
                
                # Fragment efficiency (unique fragments)
                cur.execute("SELECT COUNT(DISTINCT fragment_id) FROM model_call_cache")
                unique_fragments = cur.fetchone()[0]
        
        return {
            "total_calls": total_calls,
            "total_cost_usd": float(total_cost),
            "avg_latency_ms": float(avg_latency),
            "unique_models": unique_models,
            "unique_fragments": unique_fragments,
            "dedup_efficiency": (
                (1 - (unique_fragments / total_calls)) * 100 if total_calls > 0 else 0
            ),
        }
    
    async def get_fragment_bytes(self, fragment_id: str) -> Optional[bytes]:
        """Retrieve raw model output bytes from CAS.
        
        Used for reconstruction and verification.
        
        Args:
            fragment_id: Fragment ID (content hash)
            
        Returns:
            Raw bytes if found, None otherwise
        """
        with self.connection_pool.connection() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    "SELECT bytes FROM ikam_fragments WHERE id = %s",
                    (fragment_id,)
                )
                row = cur.fetchone()
        
        return row[0] if row else None
    
    async def clear_cache(self) -> None:
        """Clear all model call cache entries (for testing).
        
        WARNING: This is destructive and should only be used in test environments.
        """
        with self.connection_pool.connection() as cx:
            with cx.cursor() as cur:
                cur.execute("DELETE FROM model_call_cache")
                cx.commit()
        
        logger.warning("Model call cache cleared (test environment only)")


# Database Schema Helper

def create_model_call_cache_schema(cx: psycopg.Connection[Any]) -> None:
    """Create model_call_cache table if not exists.
    
    Call this during database initialization/migration.
    
    Args:
        cx: Database connection
    """
    with cx.cursor() as cur:
        cur.execute(
            """
            CREATE TABLE IF NOT EXISTS model_call_cache (
                cache_key_id TEXT PRIMARY KEY,
                model TEXT NOT NULL,
                prompt_hash TEXT NOT NULL,
                seed INTEGER,
                fragment_id TEXT NOT NULL REFERENCES ikam_fragments(id),
                artifact_id TEXT,
                function_id TEXT NOT NULL,
                execution_id TEXT NOT NULL,
                cost_usd FLOAT NOT NULL DEFAULT 0.0,
                latency_ms FLOAT NOT NULL DEFAULT 0.0,
                output_hash TEXT NOT NULL,
                output_length INTEGER NOT NULL DEFAULT 0,
                cached_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                CONSTRAINT valid_cost CHECK (cost_usd >= 0.0),
                CONSTRAINT valid_latency CHECK (latency_ms >= 0.0)
            );
            
            CREATE INDEX IF NOT EXISTS idx_model_call_cache_model_prompt_seed 
                ON model_call_cache(model, prompt_hash, seed);
            
            CREATE INDEX IF NOT EXISTS idx_model_call_cache_artifact_id 
                ON model_call_cache(artifact_id);
            
            CREATE INDEX IF NOT EXISTS idx_model_call_cache_cached_at 
                ON model_call_cache(cached_at DESC);
            """
        )
        cx.commit()
    
    logger.info("Created model_call_cache schema")
