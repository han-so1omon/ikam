from __future__ import annotations

import asyncio
from abc import ABC, abstractmethod
from datetime import datetime
from typing import Dict, List, Optional

from .models import AgentInfo, AgentStatus


class RegistryStore(ABC):
    """Abstract storage for the agent registry."""

    @abstractmethod
    async def register(self, agent: AgentInfo) -> None:
        ...

    @abstractmethod
    async def unregister(self, agent_id: str) -> None:
        ...

    @abstractmethod
    async def get(self, agent_id: str) -> Optional[AgentInfo]:
        ...

    @abstractmethod
    async def list(self) -> List[AgentInfo]:
        ...

    @abstractmethod
    async def update(self, agent_id: str, **fields) -> Optional[AgentInfo]:
        ...

    @abstractmethod
    async def heartbeat(self, agent_id: str) -> Optional[AgentInfo]:
        ...


class InMemoryRegistryStore(RegistryStore):
    """Simple in-memory store with an asyncio lock for concurrency safety."""

    def __init__(self) -> None:
        self._agents: Dict[str, AgentInfo] = {}
        self._lock = asyncio.Lock()

    async def register(self, agent: AgentInfo) -> None:
        async with self._lock:
            self._agents[agent.agent_id] = agent

    async def unregister(self, agent_id: str) -> None:
        async with self._lock:
            self._agents.pop(agent_id, None)

    async def get(self, agent_id: str) -> Optional[AgentInfo]:
        async with self._lock:
            return self._agents.get(agent_id)

    async def list(self) -> List[AgentInfo]:
        async with self._lock:
            return list(self._agents.values())

    async def update(self, agent_id: str, **fields) -> Optional[AgentInfo]:
        async with self._lock:
            agent = self._agents.get(agent_id)
            if not agent:
                return None
            data = agent.model_dump()
            data.update(fields)
            updated = AgentInfo(**data)
            self._agents[agent_id] = updated
            return updated

    async def heartbeat(self, agent_id: str) -> Optional[AgentInfo]:
        async with self._lock:
            agent = self._agents.get(agent_id)
            if not agent:
                return None
            agent = agent.model_copy(update={
                "last_heartbeat": datetime.utcnow(),
                # Mark unknown agents healthy on heartbeat by default
                "status": AgentStatus.HEALTHY if agent.status == AgentStatus.UNKNOWN else agent.status,
            })
            self._agents[agent_id] = agent
            return agent
