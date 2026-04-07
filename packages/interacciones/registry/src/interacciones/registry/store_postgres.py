from __future__ import annotations

import json
from datetime import datetime, timezone
from typing import List, Optional

import asyncpg  # type: ignore

from .models import AgentInfo, AgentStatus
from .store import RegistryStore


class PostgresRegistryStore(RegistryStore):
    """PostgreSQL-backed registry store using asyncpg.

    Schema is defined in `packages/interacciones/registry/schema.sql`.
    """

    def __init__(self, pool: asyncpg.Pool) -> None:
        self.pool = pool

    @staticmethod
    def _row_to_agent(row: asyncpg.Record | None) -> Optional[AgentInfo]:
        if not row:
            return None
        # Coerce invalid status strings to UNKNOWN to avoid exploding on legacy values
        try:
            status = AgentStatus(row["status"]) if row["status"] else AgentStatus.UNKNOWN
        except Exception:
            status = AgentStatus.UNKNOWN
        data = {
            "agent_id": row["agent_id"],
            "display_name": row["display_name"],
            "status": status,
            "capabilities": json.loads(row["capabilities"]) if row["capabilities"] else {},
            "url": row["url"],
            "meta": json.loads(row["meta"]) if row["meta"] else {},
            # Default to now() when registered_at is missing or null to satisfy model requirements
            "registered_at": row["registered_at"] or datetime.now(timezone.utc),
            "last_heartbeat": row["last_heartbeat"],
            "in_flight": row["in_flight"] or 0,
        }
        return AgentInfo(**data)

    async def register(self, agent: AgentInfo) -> None:
        async with self.pool.acquire() as cx:
            await cx.execute(
                """
                INSERT INTO agent_registry (
                    agent_id, display_name, status, capabilities, url, meta, registered_at, last_heartbeat, in_flight
                ) VALUES ($1, $2, $3, $4::jsonb, $5, $6::jsonb, $7, $8, $9)
                ON CONFLICT (agent_id) DO UPDATE SET
                    display_name = EXCLUDED.display_name,
                    status = EXCLUDED.status,
                    capabilities = EXCLUDED.capabilities,
                    url = EXCLUDED.url,
                    meta = EXCLUDED.meta,
                    last_heartbeat = EXCLUDED.last_heartbeat,
                    in_flight = EXCLUDED.in_flight
                """,
                agent.agent_id,
                agent.display_name,
                agent.status.value,
                json.dumps(agent.capabilities.model_dump()),
                agent.url,
                json.dumps(agent.meta),
                agent.registered_at,
                agent.last_heartbeat,
                agent.in_flight,
            )

    async def unregister(self, agent_id: str) -> None:
        async with self.pool.acquire() as cx:
            await cx.execute("DELETE FROM agent_registry WHERE agent_id=$1", agent_id)

    async def get(self, agent_id: str) -> Optional[AgentInfo]:
        async with self.pool.acquire() as cx:
            row = await cx.fetchrow("SELECT * FROM agent_registry WHERE agent_id=$1", agent_id)
            return self._row_to_agent(row)

    async def list(self) -> List[AgentInfo]:
        async with self.pool.acquire() as cx:
            rows = await cx.fetch("SELECT * FROM agent_registry ORDER BY agent_id")
            return [self._row_to_agent(r) for r in rows if r]

    async def update(self, agent_id: str, **fields) -> Optional[AgentInfo]:
        # Read-modify-write for simplicity
        current = await self.get(agent_id)
        if not current:
            return None
        updated = current.model_copy(update=fields)
        await self.register(updated)
        return updated

    async def heartbeat(self, agent_id: str) -> Optional[AgentInfo]:
        now = datetime.now(timezone.utc)
        async with self.pool.acquire() as cx:
            row = await cx.fetchrow(
                """
                UPDATE agent_registry
                SET last_heartbeat=$2,
                    status = CASE 
                        WHEN status IN ('unknown', 'unhealthy', 'degraded') THEN 'healthy' 
                        ELSE status 
                    END
                WHERE agent_id=$1
                RETURNING *
                """,
                agent_id,
                now,
            )
        return self._row_to_agent(row)
