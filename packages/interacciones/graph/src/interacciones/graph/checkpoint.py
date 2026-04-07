"""Checkpoint protocol and in-memory implementation for pausing/resuming graphs."""
from __future__ import annotations

import json
from typing import Any, Mapping, Optional, Protocol, Tuple


CheckpointState = Tuple[Mapping[str, Any], str]


class Checkpoint(Protocol):
    async def save(self, execution_id: str, state: Mapping[str, Any], next_node: str) -> None:
        """Persist the state and the next node to execute for this execution id."""
        ...

    async def load(self, execution_id: str) -> Optional[CheckpointState]:
        """Retrieve the saved (state, next_node) for this execution id, or None if missing."""
        ...


class InMemoryCheckpoint:
    """Simple in-memory checkpoint store for testing and local runs."""

    def __init__(self) -> None:
        self._data: dict[str, CheckpointState] = {}

    async def save(self, execution_id: str, state: Mapping[str, Any], next_node: str) -> None:
        self._data[execution_id] = (dict(state), next_node)

    async def load(self, execution_id: str) -> Optional[CheckpointState]:
        return self._data.get(execution_id)


class PostgresCheckpoint:
    """PostgreSQL-backed checkpoint store for production use.

    Requires asyncpg connection pool. Stores state as JSONB in graph_checkpoints table.

    Schema:
        CREATE TABLE IF NOT EXISTS graph_checkpoints (
            execution_id TEXT PRIMARY KEY,
            state JSONB NOT NULL,
            next_node TEXT NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        );

    Example:
        >>> import asyncpg
        >>> pool = await asyncpg.create_pool("postgresql://user:pass@localhost/db")
        >>> cp = PostgresCheckpoint(pool)
        >>> await cp.save("exec-1", {"x": 1}, "node2")
    """

    def __init__(self, pool: Any) -> None:
        """Initialize with asyncpg connection pool."""
        self._pool = pool

    async def save(self, execution_id: str, state: Mapping[str, Any], next_node: str) -> None:
        """Persist checkpoint to database (upsert)."""
        state_json = json.dumps(state)
        async with self._pool.acquire() as conn:
            await conn.execute(
                """
                INSERT INTO graph_checkpoints (execution_id, state, next_node, created_at, updated_at)
                VALUES ($1, $2::jsonb, $3, NOW(), NOW())
                ON CONFLICT (execution_id)
                DO UPDATE SET
                    state = EXCLUDED.state,
                    next_node = EXCLUDED.next_node,
                    updated_at = NOW()
                """,
                execution_id,
                state_json,
                next_node,
            )

    async def load(self, execution_id: str) -> Optional[CheckpointState]:
        """Retrieve checkpoint from database."""
        async with self._pool.acquire() as conn:
            row = await conn.fetchrow(
                "SELECT state, next_node FROM graph_checkpoints WHERE execution_id = $1",
                execution_id,
            )
            if row is None:
                return None
            state = json.loads(row["state"])
            next_node = row["next_node"]
            return (state, next_node)
