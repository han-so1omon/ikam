"""Invocation edges: function → model call dependency graph.

Responsibilities:
- Record which generated function produced a given model call output
- Enable traversal from function → model output (cache) → artifact reconstruction
- Enforce deterministic, idempotent edges (one edge per function+cache_key)

Schema:
    invocation_edges(
        edge_id TEXT PRIMARY KEY,
        function_id TEXT NOT NULL,
        cache_key_id TEXT NOT NULL REFERENCES model_call_cache(cache_key_id),
        fragment_id TEXT NOT NULL REFERENCES ikam_fragments(id),
        model TEXT NOT NULL,
        prompt_hash TEXT NOT NULL,
        seed INTEGER,
        created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
        UNIQUE (function_id, cache_key_id)
    )

Indexes:
- idx_invocation_edges_function (function_id, created_at DESC)
- idx_invocation_edges_cache (cache_key_id, created_at DESC)
- idx_invocation_edges_fragment (fragment_id)

Determinism:
- Edge key = (function_id, cache_key_id) to prevent duplicates
- Supports replay of invocation graph without re-running models
"""

from __future__ import annotations

import logging
from datetime import datetime
from typing import List, Optional
from uuid import uuid4

import psycopg
from pydantic import BaseModel, ConfigDict, Field

from .model_call_cache import ModelCallCacheFragment

logger = logging.getLogger(__name__)


class InvocationEdge(BaseModel):
    """A single invocation edge linking a function to a cached model output."""

    model_config = ConfigDict(arbitrary_types_allowed=True)

    edge_id: str = Field(default_factory=lambda: str(uuid4()))
    function_id: str = Field(..., description="Generated function identifier")
    cache_key_id: str = Field(..., description="Cache key for the model call")
    fragment_id: str = Field(..., description="CAS fragment id for the model output")
    model: str = Field(..., description="Model name used for the call")
    prompt_hash: str = Field(..., description="Hash of the input prompt")
    seed: Optional[int] = Field(None, description="Deterministic seed if provided")
    created_at: datetime = Field(default_factory=datetime.utcnow)


class InvocationGraph:
    """Manager for invocation edges (function → model call outputs)."""

    def __init__(self, connection_pool: psycopg.ConnectionPool):
        self.connection_pool = connection_pool
        logger.info("InvocationGraph initialized")

    async def add_edge(
        self,
        function_id: str,
        cache_fragment: ModelCallCacheFragment,
    ) -> InvocationEdge:
        """Add an invocation edge (idempotent on function_id + cache_key_id)."""
        edge = InvocationEdge(
            function_id=function_id,
            cache_key_id=cache_fragment.cache_key_id,
            fragment_id=cache_fragment.fragment_id,
            model=cache_fragment.model,
            prompt_hash=cache_fragment.prompt_hash,
            seed=cache_fragment.seed,
        )

        with self.connection_pool.connection() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    """
                    INSERT INTO invocation_edges (
                        edge_id, function_id, cache_key_id, fragment_id, model, prompt_hash, seed, created_at
                    ) VALUES (%s, %s, %s, %s, %s, %s, %s, %s)
                    ON CONFLICT (function_id, cache_key_id) DO NOTHING
                    RETURNING edge_id, function_id, cache_key_id, fragment_id, model, prompt_hash, seed, created_at
                    """,
                    (
                        edge.edge_id,
                        edge.function_id,
                        edge.cache_key_id,
                        edge.fragment_id,
                        edge.model,
                        edge.prompt_hash,
                        edge.seed,
                        edge.created_at,
                    ),
                )
                row = cur.fetchone()

                # If edge already exists, fetch it for deterministic return
                if row is None:
                    cur.execute(
                        """
                        SELECT edge_id, function_id, cache_key_id, fragment_id, model, prompt_hash, seed, created_at
                        FROM invocation_edges
                        WHERE function_id = %s AND cache_key_id = %s
                        """,
                        (function_id, cache_fragment.cache_key_id),
                    )
                    row = cur.fetchone()

            cx.commit()

        if row is None:
            raise RuntimeError("Invocation edge could not be created or fetched")

        return self._row_to_edge(row)

    async def get_edges_for_function(self, function_id: str) -> List[InvocationEdge]:
        """List all edges for a function (ordered by created_at)."""
        with self.connection_pool.connection() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    """
                    SELECT edge_id, function_id, cache_key_id, fragment_id, model, prompt_hash, seed, created_at
                    FROM invocation_edges
                    WHERE function_id = %s
                    ORDER BY created_at ASC
                    """,
                    (function_id,),
                )
                rows = cur.fetchall()

        return [self._row_to_edge(r) for r in rows]

    async def get_edges_for_cache_key(self, cache_key_id: str) -> List[InvocationEdge]:
        """List all edges pointing to a cache key (ordered by created_at)."""
        with self.connection_pool.connection() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    """
                    SELECT edge_id, function_id, cache_key_id, fragment_id, model, prompt_hash, seed, created_at
                    FROM invocation_edges
                    WHERE cache_key_id = %s
                    ORDER BY created_at ASC
                    """,
                    (cache_key_id,),
                )
                rows = cur.fetchall()

        return [self._row_to_edge(r) for r in rows]

    async def remove_edge(self, edge_id: str) -> None:
        """Remove a specific edge."""
        with self.connection_pool.connection() as cx:
            with cx.cursor() as cur:
                cur.execute("DELETE FROM invocation_edges WHERE edge_id = %s", (edge_id,))
            cx.commit()

    async def clear(self) -> None:
        """Delete all invocation edges (test utility)."""
        with self.connection_pool.connection() as cx:
            with cx.cursor() as cur:
                cur.execute("DELETE FROM invocation_edges")
            cx.commit()

    async def get_graph_stats(self) -> dict[str, int]:
        """Return basic graph statistics."""
        with self.connection_pool.connection() as cx:
            with cx.cursor() as cur:
                cur.execute("SELECT COUNT(*) FROM invocation_edges")
                total_edges = cur.fetchone()[0]

                cur.execute("SELECT COUNT(DISTINCT function_id) FROM invocation_edges")
                functions = cur.fetchone()[0]

                cur.execute("SELECT COUNT(DISTINCT cache_key_id) FROM invocation_edges")
                cache_keys = cur.fetchone()[0]

                cur.execute("SELECT COUNT(DISTINCT model) FROM invocation_edges")
                models = cur.fetchone()[0]

        return {
            "total_edges": total_edges,
            "functions": functions,
            "cache_keys": cache_keys,
            "models": models,
        }

    @staticmethod
    def _row_to_edge(row: tuple) -> InvocationEdge:
        return InvocationEdge(
            edge_id=row[0],
            function_id=row[1],
            cache_key_id=row[2],
            fragment_id=row[3],
            model=row[4],
            prompt_hash=row[5],
            seed=row[6],
            created_at=row[7],
        )


def create_invocation_edges_schema(connection: psycopg.Connection) -> None:
    """Create invocation_edges table and indexes if they do not exist."""
    with connection.cursor() as cur:
        cur.execute(
            """
            CREATE TABLE IF NOT EXISTS invocation_edges (
                edge_id TEXT PRIMARY KEY,
                function_id TEXT NOT NULL,
                cache_key_id TEXT NOT NULL REFERENCES model_call_cache(cache_key_id),
                fragment_id TEXT NOT NULL REFERENCES ikam_fragments(id),
                model TEXT NOT NULL,
                prompt_hash TEXT NOT NULL,
                seed INTEGER,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                UNIQUE (function_id, cache_key_id)
            )
            """
        )
        cur.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_invocation_edges_function
                ON invocation_edges (function_id, created_at DESC)
            """
        )
        cur.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_invocation_edges_cache
                ON invocation_edges (cache_key_id, created_at DESC)
            """
        )
        cur.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_invocation_edges_fragment
                ON invocation_edges (fragment_id)
            """
        )
    connection.commit()
    logger.info("invocation_edges schema ensured")
