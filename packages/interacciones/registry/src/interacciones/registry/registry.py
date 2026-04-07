from __future__ import annotations

from typing import List, Optional

from .matcher import CapabilityMatcher
from .models import AgentInfo, AgentMatch, MatchQuery, AgentStatus
from .store import InMemoryRegistryStore, RegistryStore


class AgentRegistry:
    """Facade for registering agents and finding matches.

    By default uses an in-memory store; provide a custom store for persistence.
    """

    def __init__(self, store: Optional[RegistryStore] = None) -> None:
        self.store = store or InMemoryRegistryStore()
        self.matcher = CapabilityMatcher()

    # Registration and lifecycle
    async def register(self, agent: AgentInfo) -> None:
        await self.store.register(agent)

    async def unregister(self, agent_id: str) -> None:
        await self.store.unregister(agent_id)

    async def heartbeat(self, agent_id: str) -> Optional[AgentInfo]:
        return await self.store.heartbeat(agent_id)

    async def set_status(self, agent_id: str, status: AgentStatus) -> Optional[AgentInfo]:
        return await self.store.update(agent_id, status=status)

    async def increment_in_flight(self, agent_id: str, delta: int = 1) -> Optional[AgentInfo]:
        agent = await self.store.get(agent_id)
        if not agent:
            return None
        new_val = max(0, agent.in_flight + delta)
        return await self.store.update(agent_id, in_flight=new_val)

    # Queries
    async def get(self, agent_id: str) -> Optional[AgentInfo]:
        return await self.store.get(agent_id)

    async def list(self) -> List[AgentInfo]:
        return await self.store.list()

    async def match(self, *, domain: Optional[str] = None, action: Optional[str] = None, tags: Optional[List[str]] = None, require_healthy: bool = True, limit: int = 5) -> List[AgentMatch]:
        agents = await self.list()
        q = MatchQuery(domain=domain, action=action, tags=tags or [], require_healthy=require_healthy, limit=limit)
        return self.matcher.rank(agents, q)
