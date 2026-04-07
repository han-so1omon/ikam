"""Function storage service with CAS deduplication.

Provides storage, retrieval, and deduplication for generated functions using:
- Code canonicalization (modelado.core.canonicalize)
- BLAKE3 content addressing
- PostgreSQL backend with CAS semantics

Usage:
    storage = FunctionStorageService(db_url)
    
    # Store generated function
    record = await storage.store_function(
        code=generated_code,
        metadata=metadata,
    )
    
    # Retrieve by content hash
    function = await storage.get_by_hash(record.content_hash)
    
    # Get storage statistics
    stats = await storage.get_storage_stats()
    print(f"Deduplication: {stats.storage_savings_percent}%")
"""

from __future__ import annotations

import hashlib
import json
import logging
from datetime import datetime
from pathlib import Path
from typing import Any, Dict, List, Optional

import psycopg
from psycopg.rows import dict_row
from psycopg.types.json import Json
from psycopg_pool import ConnectionPool

try:
    import blake3
    BLAKE3_AVAILABLE = True
except ImportError:
    BLAKE3_AVAILABLE = False

from .canonicalize import canonicalize_function, CanonicalizedCode
from .execution_context import ExecutionPolicyViolation, get_execution_context, require_write_scope
from .function_storage import (
    GeneratedFunctionMetadata,
    GeneratedFunctionRecord,
    FunctionStorageStats,
)

logger = logging.getLogger(__name__)

MIGRATION_PATH = Path(__file__).resolve().parent.parent / "migrations" / "015_add_generated_functions.sql"


class FunctionStorageService:
    """Service for storing and retrieving generated functions with CAS deduplication.
    
    Mathematical guarantees:
    1. Idempotent storage: store(f) always returns same function_id for equivalent code
    2. CAS property: content_hash uniquely identifies canonical code
    3. Storage monotonicity: Total storage ≤ sum of individual function sizes
    4. Provenance preservation: All generation metadata persists with function
    """
    
    def __init__(self, connection_string: Optional[str] = None, use_blake3: bool = True):
        """Initialize function storage service.
        
        Args:
            connection_string: PostgreSQL connection string (optional for in-memory mode)
            use_blake3: Use BLAKE3 for hashing (faster); falls back to SHA256 if unavailable
        """
        self.connection_string = connection_string
        self.use_blake3 = use_blake3 and BLAKE3_AVAILABLE

        # In-memory storage for testing/development
        self._memory_store: Dict[str, GeneratedFunctionRecord] = {}
        # In-memory alias mapping (caller-provided function_id -> canonical CAS id)
        self._memory_aliases: Dict[str, str] = {}
        self._storage_mode = "memory" if connection_string is None else "postgres"
        self._stats = {
            "total_functions_generated": 0,
            "duplicate_count": 0,
            "raw_storage_bytes": 0,
            "deduplicated_storage_bytes": 0,
        }

        self.connection_pool: Optional[ConnectionPool] = None
        if self._storage_mode == "postgres":
            # Small pool for function CAS writes/reads; reuse dict_row for convenient mapping
            self.connection_pool = ConnectionPool(
                conninfo=self.connection_string,
                min_size=1,
                max_size=5,
                kwargs={"row_factory": dict_row},
                open=True,
            )
        
        logger.info(
            f"FunctionStorageService initialized (mode={self._storage_mode}, "
            f"blake3={'enabled' if self.use_blake3 else 'disabled'})"
        )
    
    def close(self) -> None:
        """Close underlying connection pool (PostgreSQL mode)."""

        if self.connection_pool is not None:
            try:
                self.connection_pool.close()
            except Exception:  # pragma: no cover - defensive close
                logger.warning("Failed closing FunctionStorageService pool", exc_info=True)

    async def store_function(
        self,
        code: str,
        metadata: GeneratedFunctionMetadata,
        cache_key: Optional[str] = None,
    ) -> GeneratedFunctionRecord:
        """Store generated function with canonicalization and CAS deduplication.
        
        Args:
            code: Generated Python function code
            metadata: Generation metadata (intent, strategy, provenance)
            cache_key: Optional semantic cache key (intent + params hash)
            
        Returns:
            GeneratedFunctionRecord with storage metadata
            
        Mathematical property:
            If canonicalize(code1) == canonicalize(code2), returns same function_id
        """
        # Step 1: Canonicalize code
        canonical_result = canonicalize_function(code, use_blake3=self.use_blake3)
        
        # Step 2: Compute canonical CAS function id (short prefix of content hash)
        cas_function_id = f"gfn_{canonical_result.content_hash[:16]}"

        # Optional caller-facing id (primarily for in-memory tests)
        requested_function_id = metadata.function_id or cas_function_id
        
        # Step 3: Check if already stored (CAS deduplication)
        existing = await self.get_by_hash(canonical_result.content_hash)

        # Track total operations and raw bytes regardless of dedup
        self._stats["total_functions_generated"] += 1
        self._stats["raw_storage_bytes"] += len(code.encode())

        if existing:
            logger.info(
                f"Function deduplicated: {cas_function_id} "
                f"(original_key={existing.storage_key})"
            )
            # Update stored execution count (stored record stays deduplicated=False)
            existing.execution_count += 1
            existing.cache_key = cache_key or existing.cache_key
            await self._update_record(existing)

            # Create a deduplication view for the caller without mutating the stored record
            # If the caller supplied a function_id in memory mode, remember it as an alias.
            if self._storage_mode == "memory" and requested_function_id != existing.function_id:
                self._memory_aliases[requested_function_id] = existing.function_id

            dedup_record = GeneratedFunctionRecord(
                function_id=(
                    requested_function_id
                    if self._storage_mode == "memory"
                    else existing.function_id
                ),
                content_hash=existing.content_hash,
                canonical_code=existing.canonical_code,
                original_code=code,
                transformations_applied=self._normalize_transformations(canonical_result.transformations),
                is_semantically_equivalent=canonical_result.is_semantically_equivalent,
                metadata=metadata,
                stored_at=existing.stored_at,
                storage_key=existing.storage_key,
                deduplicated=True,
                original_storage_key=existing.storage_key,
                cache_key=cache_key or existing.cache_key,
                execution_count=existing.execution_count,
            )

            self._stats["duplicate_count"] += 1
            return dedup_record
        
        # Step 4: Create new storage record
        record = GeneratedFunctionRecord(
            function_id=cas_function_id,
            content_hash=canonical_result.content_hash,
            canonical_code=canonical_result.canonical_code,
            original_code=code,
            transformations_applied=self._normalize_transformations(canonical_result.transformations),
            is_semantically_equivalent=canonical_result.is_semantically_equivalent,
            metadata=metadata,
            stored_at=datetime.utcnow(),
            storage_key=cas_function_id,  # Use CAS id as storage key
            deduplicated=False,
            cache_key=cache_key,
            execution_count=1,
        )

        # Accumulate deduplicated storage bytes (canonical form stored once)
        self._stats["deduplicated_storage_bytes"] += len(record.canonical_code.encode())
        
        # Step 5: Store in backend
        await self._store_record(record)

        # In-memory mode: allow caller-provided ids to resolve to the canonical CAS record.
        if self._storage_mode == "memory" and requested_function_id != cas_function_id:
            self._memory_aliases[requested_function_id] = cas_function_id
            # Return a caller-facing view record with the requested id.
            record = GeneratedFunctionRecord(
                function_id=requested_function_id,
                content_hash=record.content_hash,
                canonical_code=record.canonical_code,
                original_code=record.original_code,
                transformations_applied=record.transformations_applied,
                is_semantically_equivalent=record.is_semantically_equivalent,
                metadata=record.metadata,
                stored_at=record.stored_at,
                storage_key=record.storage_key,
                deduplicated=record.deduplicated,
                original_storage_key=record.original_storage_key,
                cache_key=record.cache_key,
                execution_count=record.execution_count,
            )
        
        logger.info(
            f"Function stored: {cas_function_id} "
            f"(transformations={len(canonical_result.transformations)})"
        )
        
        return record
    
    async def get_by_hash(self, content_hash: str) -> Optional[GeneratedFunctionRecord]:
        """Retrieve function by content hash (CAS lookup).
        
        Args:
            content_hash: BLAKE3 or SHA256 hash of canonical code
            
        Returns:
            GeneratedFunctionRecord if found, None otherwise
        """
        if self._storage_mode == "memory":
            for record in self._memory_store.values():
                if record.content_hash == content_hash:
                    return record
            return None
        else:
            with self.connection_pool.connection() as cx:
                with cx.cursor() as cur:
                    cur.execute(
                        """
                        SELECT * FROM ikam_generated_functions WHERE content_hash = %s
                        """,
                        (content_hash,),
                    )
                    row = cur.fetchone()
            return self._row_to_record(row) if row else None
    
    async def get_by_function_id(self, function_id: str) -> Optional[GeneratedFunctionRecord]:
        """Retrieve function by function_id.
        
        Args:
            function_id: Short function ID (e.g., gfn_abc123def456)
            
        Returns:
            GeneratedFunctionRecord if found, None otherwise
        """
        if self._storage_mode == "memory":
            direct = self._memory_store.get(function_id)
            if direct is not None:
                return direct
            alias = self._memory_aliases.get(function_id)
            if alias is None:
                return None
            return self._memory_store.get(alias)
        else:
            with self.connection_pool.connection() as cx:
                with cx.cursor() as cur:
                    cur.execute(
                        "SELECT * FROM ikam_generated_functions WHERE function_id = %s",
                        (function_id,),
                    )
                    row = cur.fetchone()
            return self._row_to_record(row) if row else None
    
    async def get_by_cache_key(self, cache_key: str) -> Optional[GeneratedFunctionRecord]:
        """Retrieve function by semantic cache key.
        
        Enables fast lookup for repeated semantic intents.
        
        Args:
            cache_key: Semantic cache key (intent + params hash)
            
        Returns:
            GeneratedFunctionRecord if found, None otherwise
        """
        if self._storage_mode == "memory":
            for record in self._memory_store.values():
                if record.cache_key == cache_key:
                    return record
            return None
        else:
            with self.connection_pool.connection() as cx:
                with cx.cursor() as cur:
                    cur.execute(
                        "SELECT * FROM ikam_generated_functions WHERE cache_key = %s",
                        (cache_key,),
                    )
                    row = cur.fetchone()
            return self._row_to_record(row) if row else None
    
    async def get_storage_stats(self) -> FunctionStorageStats:
        """Calculate storage statistics for deduplication analysis.
        
        Returns:
            FunctionStorageStats with deduplication metrics
            
        Validates storage monotonicity: Δ(N) = raw_bytes - dedup_bytes ≥ 0
        """
        if self._storage_mode == "memory":
            raw_bytes = self._stats["raw_storage_bytes"]
            dedup_bytes = self._stats["deduplicated_storage_bytes"]
            savings_bytes = raw_bytes - dedup_bytes
            savings_percent = (savings_bytes / raw_bytes * 100) if raw_bytes > 0 else 0.0
            monotonicity_delta = max(savings_bytes, 0)
            unique_count = len(self._memory_store)
            duplicate_count = self._stats["duplicate_count"]

            return FunctionStorageStats(
                total_functions_generated=self._stats["total_functions_generated"],
                unique_functions_stored=unique_count,
                duplicate_count=duplicate_count,
                storage_savings_bytes=savings_bytes,
                storage_savings_percent=round(savings_percent, 2),
                raw_storage_bytes=raw_bytes,
                deduplicated_storage_bytes=dedup_bytes,
                monotonicity_delta=monotonicity_delta,
            )
        else:
            # Derive stats from base table to include execution_count for duplicates
            with self.connection_pool.connection() as cx:
                with cx.cursor() as cur:
                    cur.execute(
                        """
                        SELECT function_id, execution_count,
                               LENGTH(original_code) AS original_len,
                               LENGTH(canonical_code) AS canonical_len
                        FROM ikam_generated_functions
                        """
                    )
                    rows = cur.fetchall() or []

            total_functions_generated = 0
            raw_bytes = 0
            dedup_bytes = 0
            duplicate_count = 0

            for row in rows:
                exec_count = row.get("execution_count") or 0
                original_len = row.get("original_len") or 0
                canonical_len = row.get("canonical_len") or 0

                total_functions_generated += exec_count
                raw_bytes += original_len * max(exec_count, 1)
                dedup_bytes += canonical_len
                duplicate_count += max(exec_count - 1, 0)

            savings_bytes = raw_bytes - dedup_bytes
            monotonicity_delta = max(savings_bytes, 0)
            savings_percent = (savings_bytes / raw_bytes * 100) if raw_bytes > 0 else 0.0

            return FunctionStorageStats(
                total_functions_generated=total_functions_generated,
                unique_functions_stored=len(rows),
                duplicate_count=duplicate_count,
                storage_savings_bytes=int(savings_bytes),
                storage_savings_percent=float(round(savings_percent, 2)),
                raw_storage_bytes=int(raw_bytes),
                deduplicated_storage_bytes=int(dedup_bytes),
                monotonicity_delta=int(monotonicity_delta),
            )
    
    async def list_functions(
        self,
        limit: int = 100,
        offset: int = 0,
        user_intent: Optional[str] = None,
    ) -> List[GeneratedFunctionRecord]:
        """List stored functions with optional filtering.
        
        Args:
            limit: Maximum functions to return
            offset: Pagination offset
            user_intent: Filter by user intent
            
        Returns:
            List of GeneratedFunctionRecord objects
        """
        if self._storage_mode == "memory":
            records = list(self._memory_store.values())
            
            # Filter by user intent if provided
            if user_intent:
                records = [
                    r for r in records
                    if r.metadata and r.metadata.user_intent == user_intent
                ]
            
            # Apply pagination
            return records[offset:offset + limit]
        else:
            clauses = ["TRUE"]
            params: List[Any] = []
            if user_intent:
                clauses.append("parameters ->> 'user_intent' = %s")
                params.append(user_intent)

            where_sql = " AND ".join(clauses)
            with self.connection_pool.connection() as cx:
                with cx.cursor() as cur:
                    cur.execute(
                        f"""
                        SELECT * FROM ikam_generated_functions
                        WHERE {where_sql}
                        ORDER BY stored_at DESC
                        LIMIT %s OFFSET %s
                        """,
                        (*params, limit, offset),
                    )
                    rows = cur.fetchall()

            return [self._row_to_record(row) for row in rows]
    
    # Private methods

    def _require_authorized_write(self, operation: str) -> None:
        ctx = get_execution_context()
        if ctx is None:
            raise ExecutionPolicyViolation(
                f"Execution policy violation: '{operation}' requires an active execution context"
            )
        if ctx.actor_id is None:
            require_write_scope(operation)
    
    async def _store_record(self, record: GeneratedFunctionRecord) -> None:
        """Store record in backend."""
        if self._storage_mode == "memory":
            self._memory_store[record.function_id] = record
        else:
            self._require_authorized_write("ikam.generated_functions.store_record")
            metadata_json = metadata_to_json(record.metadata)
            with self.connection_pool.connection() as cx:
                with cx.cursor() as cur:
                    cur.execute(
                        """
                        INSERT INTO ikam_generated_functions (
                            function_id, content_hash, canonical_code, original_code,
                            transformations_applied, is_semantically_equivalent,
                            semantic_intent, generation_strategy, model_name,
                            parameters, storage_key, deduplicated, cache_key,
                            execution_count, stored_at
                        ) VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
                        ON CONFLICT (content_hash) DO UPDATE SET
                            execution_count = ikam_generated_functions.execution_count + 1,
                            cache_key = COALESCE(EXCLUDED.cache_key, ikam_generated_functions.cache_key),
                            deduplicated = TRUE,
                            updated_at = NOW()
                        RETURNING *
                        """,
                        (
                            record.function_id,
                            record.content_hash,
                            record.canonical_code,
                            record.original_code,
                            Json(record.transformations_applied),
                            record.is_semantically_equivalent,
                            record.metadata.semantic_intent if record.metadata else None,
                            record.metadata.strategy if record.metadata else None,
                            record.metadata.generator_version if record.metadata else None,
                            Json(metadata_json),
                            record.storage_key,
                            record.deduplicated,
                            record.cache_key,
                            record.execution_count,
                            record.stored_at,
                        ),
                    )
                    row = cur.fetchone()
                cx.commit()

            # Align in-memory representation with persisted values (execution_count may change on conflict)
            persisted = self._row_to_record(row)
            record.execution_count = persisted.execution_count
    
    async def _update_record(self, record: GeneratedFunctionRecord) -> None:
        """Update existing record in backend."""
        if self._storage_mode == "memory":
            self._memory_store[record.function_id] = record
        else:
            self._require_authorized_write("ikam.generated_functions.update_record")
            metadata_json = metadata_to_json(record.metadata)
            with self.connection_pool.connection() as cx:
                with cx.cursor() as cur:
                    cur.execute(
                        """
                        UPDATE ikam_generated_functions
                        SET execution_count = %s,
                            cache_key = COALESCE(%s, cache_key),
                            parameters = %s,
                            semantic_intent = COALESCE(%s, semantic_intent),
                            generation_strategy = COALESCE(%s, generation_strategy),
                            model_name = COALESCE(%s, model_name),
                            updated_at = NOW()
                        WHERE function_id = %s
                        RETURNING *
                        """,
                        (
                            record.execution_count,
                            record.cache_key,
                            Json(metadata_json),
                            record.metadata.semantic_intent if record.metadata else None,
                            record.metadata.strategy if record.metadata else None,
                            record.metadata.generator_version if record.metadata else None,
                            record.function_id,
                        ),
                    )
                    cur.fetchone()
                cx.commit()
    
    def _compute_cache_key(self, intent: str, params: Optional[Dict] = None) -> str:
        """Compute semantic cache key from intent and parameters.
        
        Args:
            intent: Semantic intent string
            params: Optional parameters dictionary
            
        Returns:
            Hex-encoded hash (cache key)
        """
        # Serialize intent + params to stable JSON
        cache_input = {
            "intent": intent,
            "params": params or {},
        }
        canonical_json = json.dumps(cache_input, sort_keys=True, separators=(",", ":"))
        
        # Compute hash
        if self.use_blake3:
            return blake3.blake3(canonical_json.encode()).hexdigest()[:32]
        else:
            return hashlib.sha256(canonical_json.encode()).hexdigest()[:32]

    @staticmethod
    def _normalize_transformations(transformations: List) -> List[Dict[str, str]]:
        """Ensure transformations are recorded as list of dicts with 'type' keys."""
        normalized: List[Dict[str, str]] = []
        for t in transformations:
            if isinstance(t, dict):
                normalized.append(t)
            else:
                normalized.append({"type": str(t)})
        # Add a generic whitespace_normalized marker when we apply formatting-related transforms
        type_set = {t.get("type") for t in normalized}
        if type_set and not ("whitespace_normalized" in type_set):
            if any(
                key for key in type_set
                if key and ("normalize" in key or "format" in key or "whitespace" in key)
            ):
                normalized.append({"type": "whitespace_normalized"})
        return normalized

    @staticmethod
    def _row_to_record(row: Dict[str, Any]) -> Optional[GeneratedFunctionRecord]:
        """Convert database row to GeneratedFunctionRecord."""

        if not row:
            return None

        metadata_blob = row.get("parameters") or {}
        metadata: Optional[GeneratedFunctionMetadata] = None
        if metadata_blob:
            try:
                metadata = GeneratedFunctionMetadata(**metadata_blob)
            except Exception:
                logger.warning("Failed to deserialize metadata; returning None", exc_info=True)

        stored_at = row.get("stored_at")

        return GeneratedFunctionRecord(
            function_id=row["function_id"],
            content_hash=row["content_hash"],
            canonical_code=row["canonical_code"],
            original_code=row["original_code"],
            transformations_applied=row.get("transformations_applied") or [],
            is_semantically_equivalent=row.get("is_semantically_equivalent", True),
            metadata=metadata,
            stored_at=stored_at,
            storage_key=row.get("storage_key"),
            deduplicated=row.get("deduplicated", False),
            original_storage_key=row.get("original_storage_key"),
            cache_key=row.get("cache_key"),
            execution_count=row.get("execution_count", 0),
        )


def metadata_to_json(metadata: Optional[GeneratedFunctionMetadata]) -> Dict[str, Any]:
    """Serialize GeneratedFunctionMetadata to JSON-safe dict for storage."""

    if metadata is None:
        return {}
    return metadata.model_dump(mode="json")


def apply_function_storage_migration(cx: psycopg.Connection[Any]) -> None:
    """Apply the generated functions migration (015) against a PostgreSQL connection."""

    sql = MIGRATION_PATH.read_text()
    with cx.cursor() as cur:
        cur.execute(sql)
    cx.commit()
    logger.info("Applied function storage migration (015_add_generated_functions.sql)")
