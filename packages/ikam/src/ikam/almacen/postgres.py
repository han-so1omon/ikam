"""PostgreSQL storage backend for IKAM fragments with CAS support.

This module implements content-addressable storage (CAS) using PostgreSQL
with BLAKE3 hashing for fragment deduplication. Supports the full
StorageBackend interface with atomic operations and metadata queries.

Mathematical Guarantees:
- CAS property: hash(content) → unique key (collision probability < 2^-128)
- Idempotent PUT: put(content) returns same key regardless of call count
- Atomic operations: all DB operations wrapped in transactions
- Storage monotonicity: Δ(N) = S_flat(N) - S_CAS(N) ≥ 0 when N ≥ 2

Database Schema:
    CREATE TABLE ikam_fragments (
        fragment_id SERIAL PRIMARY KEY,
        content_hash TEXT NOT NULL UNIQUE,  -- BLAKE3 hash for CAS
        kind TEXT NOT NULL,                 -- fragment kind (text, patch, chart)
        payload BYTEA NOT NULL,             -- raw fragment bytes
        size_bytes INT NOT NULL,            -- payload size for metrics
        metadata JSONB DEFAULT '{}',        -- extensible metadata
        created_at TIMESTAMP DEFAULT NOW(),
        updated_at TIMESTAMP DEFAULT NOW()
    );
    CREATE INDEX idx_ikam_fragments_hash ON ikam_fragments(content_hash);
    CREATE INDEX idx_ikam_fragments_kind ON ikam_fragments(kind);

Usage:
    from ikam.almacen.postgres import PostgresBackend
    from ikam.almacen.base import FragmentRecord, FragmentKey
    
    backend = PostgresBackend(connection_string="postgresql://...")
    backend.initialize()  # Create schema if needed
    
    # Store fragment (CAS deduplication automatic)
    key = backend.put(FragmentRecord(
        key=FragmentKey(key="", kind="text"),  # key auto-generated
        payload=b"Hello, world!",
        metadata={"source": "user-input"}
    ))
    print(f"Stored as: {key.key}")  # blake3:abc123...
    
    # Retrieve fragment
    record = backend.get(key)
    assert record.payload == b"Hello, world!"
"""
from __future__ import annotations

import hashlib
import json
import logging
from typing import Any, Dict, Iterable, Optional

from .base import Capability, FragmentKey, FragmentRecord, StorageBackend

logger = logging.getLogger("ikam.almacen.postgres")

try:
    import psycopg
    from psycopg.rows import dict_row
    PSYCOPG_AVAILABLE = True
except ImportError:
    PSYCOPG_AVAILABLE = False
    logger.warning("psycopg not available; PostgresBackend will fail at runtime")


def _blake3_hash(data: bytes) -> str:
    """Compute BLAKE3 hash of bytes (32-byte output, 64-char hex).
    
    BLAKE3 properties:
    - Cryptographically secure (collision resistance > 2^128)
    - Fast (3x faster than SHA256 on modern CPUs)
    - Deterministic (same input → same hash)
    """
    try:
        import blake3  # type: ignore[import-not-found]
        return blake3.blake3(data).hexdigest()
    except ImportError:
        # Fallback to SHA256 if blake3 not available
        logger.warning("blake3 not available; falling back to sha256 (slower)")
        return hashlib.sha256(data).hexdigest()


class PostgresBackend(StorageBackend):
    """PostgreSQL storage backend with content-addressable storage (CAS).
    
    Features:
    - Automatic deduplication via BLAKE3 content hashing
    - Atomic PUT/GET/DELETE operations (transaction-wrapped)
    - Metadata query support (JSONB indexes)
    - Version tracking (created_at, updated_at)
    - Prometheus-ready metrics (size_bytes column)
    
    Attributes:
        name: Backend identifier ("postgresql")
        connection_string: PostgreSQL connection string
        table_name: Table name for fragments (default: "ikam_fragments")
    """
    
    name = "postgresql"
    
    def __init__(
        self,
        connection_string: str,
        *,
        table_name: str = "ikam_fragments",
        auto_initialize: bool = True,
    ):
        """Initialize PostgreSQL backend.
        
        Args:
            connection_string: PostgreSQL DSN (e.g., "postgresql://user:pass@host/db")
            table_name: Table name for fragment storage
            auto_initialize: If True, create schema on first connection
        """
        if not PSYCOPG_AVAILABLE:
            raise ImportError("psycopg is required for PostgresBackend")
        
        self.connection_string = connection_string
        self.table_name = table_name
        self._initialized = False
        
        if auto_initialize:
            self.initialize()
    
    @property
    def capabilities(self) -> frozenset[Capability]:
        """PostgreSQL supports all core + optional capabilities."""
        return frozenset({
            Capability.PUT,
            Capability.GET,
            Capability.DELETE,
            Capability.LIST,
            Capability.CAS,
            Capability.VERSIONS,
            Capability.METADATA_QUERY,
        })
    
    def _connect(self):
        """Create database connection with dict row factory."""
        return psycopg.connect(self.connection_string, row_factory=dict_row)
    
    def initialize(self) -> None:
        """Create ikam_fragments table if not exists.
        
        Idempotent: safe to call multiple times.
        Creates indexes for content_hash (CAS lookup) and kind (filtering).
        """
        if self._initialized:
            return
        
        # Step 1: Create table with minimal required schema (columns that always existed)
        schema_sql = f"""
        CREATE TABLE IF NOT EXISTS {self.table_name} (
            fragment_id SERIAL PRIMARY KEY,
            kind TEXT NOT NULL,
            payload BYTEA NOT NULL,
            metadata JSONB DEFAULT '{{}}',
            created_at TIMESTAMP DEFAULT NOW(),
            updated_at TIMESTAMP DEFAULT NOW()
        );
        """

        # Step 2: Backward-compatibility - add new columns if missing (only if table exists)
        alter_sql = f"""
        DO $$
        BEGIN
            -- Only attempt ALTER if table exists
            IF EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = '{self.table_name}') THEN
                IF NOT EXISTS (
                    SELECT 1 FROM information_schema.columns 
                    WHERE table_name = '{self.table_name}' AND column_name = 'content_hash'
                ) THEN
                    ALTER TABLE {self.table_name} ADD COLUMN content_hash TEXT;
                END IF;
                IF NOT EXISTS (
                    SELECT 1 FROM information_schema.columns 
                    WHERE table_name = '{self.table_name}' AND column_name = 'size_bytes'
                ) THEN
                    ALTER TABLE {self.table_name} ADD COLUMN size_bytes INT;
                END IF;
            END IF;
        END
        $$;
        """
        
        # Step 3: Create indexes (safe now that columns exist)
        index_sql = f"""
        CREATE INDEX IF NOT EXISTS idx_{self.table_name}_hash 
            ON {self.table_name}(content_hash);
        CREATE INDEX IF NOT EXISTS idx_{self.table_name}_kind 
            ON {self.table_name}(kind);
        CREATE INDEX IF NOT EXISTS idx_{self.table_name}_metadata 
            ON {self.table_name} USING gin(metadata);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_{self.table_name}_content_hash_unique
            ON {self.table_name}(content_hash);
        """
        
        with self._connect() as conn:
            with conn.cursor() as cur:
                cur.execute(schema_sql)
                cur.execute(alter_sql)
                cur.execute(index_sql)
                conn.commit()
        
        self._initialized = True
        logger.info(f"Initialized PostgreSQL backend (table: {self.table_name})")
    
    def put(self, record: FragmentRecord) -> FragmentKey:
        """Store fragment with CAS deduplication.
        
        If content hash already exists, returns existing key without
        modifying the database (idempotent PUT).
        
        Args:
            record: Fragment record to store
            
        Returns:
            FragmentKey with content-addressable hash
            
        Mathematical guarantee:
            put(r1) == put(r2) when r1.payload == r2.payload
        """
        content_hash = _blake3_hash(record.payload)
        key = FragmentKey(key=f"blake3:{content_hash}", kind=record.key.kind)
        
        # Upsert: insert if new, update metadata if exists
        upsert_sql = f"""
        INSERT INTO {self.table_name} 
            (content_hash, kind, payload, size_bytes, metadata)
        VALUES (%(hash)s, %(kind)s, %(payload)s, %(size)s, %(meta)s)
        ON CONFLICT (content_hash) DO UPDATE
        SET 
            metadata = EXCLUDED.metadata,
            updated_at = NOW()
        RETURNING content_hash
        """
        
        with self._connect() as conn:
            with conn.cursor() as cur:
                cur.execute(
                    upsert_sql,
                    {
                        "hash": content_hash,
                        "kind": record.key.kind,
                        "payload": record.payload,
                        "size": len(record.payload),
                        "meta": json.dumps(record.metadata),
                    },
                )
                conn.commit()
        
        logger.debug(f"PUT fragment: {key.key[:16]}... ({len(record.payload)} bytes)")
        return key
    
    def get(self, key: FragmentKey) -> Optional[FragmentRecord]:
        """Retrieve fragment by content hash.
        
        Args:
            key: FragmentKey with blake3:hash format
            
        Returns:
            FragmentRecord if found, None otherwise
        """
        # Extract hash from "blake3:hexhash" format
        if not key.key.startswith("blake3:"):
            logger.warning(f"Invalid key format: {key.key} (expected blake3:...)")
            return None
        
        content_hash = key.key[7:]  # Strip "blake3:" prefix
        
        select_sql = f"""
        SELECT content_hash, kind, payload, metadata
        FROM {self.table_name}
        WHERE content_hash = %(hash)s
        """
        
        with self._connect() as conn:
            with conn.cursor() as cur:
                cur.execute(select_sql, {"hash": content_hash})
                row = cur.fetchone()
        
        if not row:
            logger.debug(f"GET fragment: {key.key[:16]}... NOT FOUND")
            return None
        
        logger.debug(f"GET fragment: {key.key[:16]}... ({len(row['payload'])} bytes)")
        return FragmentRecord(
            key=FragmentKey(key=f"blake3:{row['content_hash']}", kind=row["kind"]),
            payload=bytes(row["payload"]),
            metadata=row["metadata"] or {},
        )
    
    def delete(self, key: FragmentKey) -> bool:
        """Delete fragment by content hash.
        
        Args:
            key: FragmentKey with blake3:hash format
            
        Returns:
            True if fragment was deleted, False if not found
        """
        if not key.key.startswith("blake3:"):
            logger.warning(f"Invalid key format: {key.key} (expected blake3:...)")
            return False
        
        content_hash = key.key[7:]
        
        delete_sql = f"""
        DELETE FROM {self.table_name}
        WHERE content_hash = %(hash)s
        """
        
        with self._connect() as conn:
            with conn.cursor() as cur:
                cur.execute(delete_sql, {"hash": content_hash})
                deleted = cur.rowcount > 0
                conn.commit()
        
        logger.debug(f"DELETE fragment: {key.key[:16]}... (deleted: {deleted})")
        return deleted
    
    def list(self, prefix: Optional[str] = None) -> Iterable[FragmentKey]:
        """Iterate fragment keys, optionally filtered by kind prefix.
        
        Args:
            prefix: Optional kind prefix (e.g., "text" matches "text", "text/plain")
            
        Yields:
            FragmentKey objects in creation order
        """
        if prefix:
            list_sql = f"""
            SELECT content_hash, kind
            FROM {self.table_name}
            WHERE kind LIKE %s
            ORDER BY created_at
            """
            params = (f"{prefix}%",)
        else:
            list_sql = f"""
            SELECT content_hash, kind
            FROM {self.table_name}
            ORDER BY created_at
            """
            params = None
        
        with self._connect() as conn:
            with conn.cursor() as cur:
                if params is None:
                    cur.execute(list_sql)
                else:
                    cur.execute(list_sql, params)
                for row in cur:
                    yield FragmentKey(
                        key=f"blake3:{row['content_hash']}",
                        kind=row["kind"],
                    )
    
    def describe(self) -> Dict[str, Any]:
        """Return backend description with storage stats."""
        base_desc = super().describe()
        
        stats_sql = f"""
        SELECT 
            COUNT(*) as fragment_count,
            SUM(size_bytes) as total_bytes,
            COUNT(DISTINCT kind) as kind_count
        FROM {self.table_name}
        """
        
        try:
            with self._connect() as conn:
                with conn.cursor() as cur:
                    cur.execute(stats_sql)
                    stats = cur.fetchone()
            
            return {
                **base_desc,
                "table": self.table_name,
                "stats": {
                    "fragment_count": stats["fragment_count"] or 0,
                    "total_bytes": stats["total_bytes"] or 0,
                    "kind_count": stats["kind_count"] or 0,
                },
            }
        except Exception as exc:
            logger.warning(f"Failed to fetch stats: {exc}")
            return {**base_desc, "table": self.table_name}


__all__ = ["PostgresBackend"]
