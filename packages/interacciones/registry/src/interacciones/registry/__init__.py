"""Interacciones Registry - Agent discovery and capability matching for multi-agent systems.

Provides:
- `AgentRegistry` — Register, discover, and route to agents
- `AgentCapability` — Agent capability metadata (domains, actions, constraints)
- `CapabilityMatcher` — Match requests to agents based on capabilities

Example:
    >>> from interacciones.registry import AgentRegistry, AgentCapability, AgentInfo
    >>> reg = AgentRegistry()
    >>> await reg.register(AgentInfo(
    ...     agent_id="econ-modeler",
    ...     capabilities=AgentCapability(domains=["economics"], actions=["analyze"]))
    ... )
    >>> matches = await reg.match(domain="economics", action="analyze")
"""

from .models import AgentCapability, AgentInfo, AgentMatch, AgentStatus, MatchQuery
from .matcher import CapabilityMatcher
from .registry import AgentRegistry
from .store import InMemoryRegistryStore, RegistryStore
from .store_postgres import PostgresRegistryStore

__all__ = [
    "AgentRegistry",
    "AgentCapability",
    "AgentInfo",
    "AgentMatch",
    "AgentStatus",
    "MatchQuery",
    "CapabilityMatcher",
    "RegistryStore",
    "InMemoryRegistryStore",
    "PostgresRegistryStore",
]
