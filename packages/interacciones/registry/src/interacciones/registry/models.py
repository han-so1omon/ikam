from __future__ import annotations

from datetime import datetime, timezone
from enum import Enum
from typing import Dict, List, Optional

from pydantic import BaseModel, Field


class AgentStatus(str, Enum):
    UNKNOWN = "unknown"
    HEALTHY = "healthy"
    DEGRADED = "degraded"
    UNHEALTHY = "unhealthy"
    DRAINING = "draining"  # not accepting new work


class AgentCapability(BaseModel):
    """Describes what an agent can do.

    - domains: high-level domains (e.g., "economics", "story", "reporting")
    - actions: verbs the agent supports (e.g., "analyze", "predict", "summarize")
    - tags: optional fine-grained labels (e.g., "finance", "saas", "series-a")
    - max_concurrency: concurrent tasks allowed (None means unbounded)
    - cost_hint: optional relative cost (lower is cheaper)
    - priority: optional priority level (higher is preferred) for tie-breaking
    - domain_weights: optional per-domain weights (higher = stronger capability)
    - action_weights: optional per-action weights (higher = stronger capability)
    - tag_weights: optional per-tag weights (higher = stronger capability)
    - min_version: optional minimum version constraint (semantic version string)
    - max_version: optional maximum version constraint (semantic version string)
    """

    domains: List[str] = Field(default_factory=list)
    actions: List[str] = Field(default_factory=list)
    tags: List[str] = Field(default_factory=list)
    max_concurrency: Optional[int] = None
    cost_hint: Optional[float] = None
    priority: int = 0
    domain_weights: Dict[str, float] = Field(default_factory=dict)
    action_weights: Dict[str, float] = Field(default_factory=dict)
    tag_weights: Dict[str, float] = Field(default_factory=dict)
    min_version: Optional[str] = None
    max_version: Optional[str] = None


class AgentInfo(BaseModel):
    """Registered agent metadata."""

    agent_id: str
    display_name: Optional[str] = None
    capabilities: AgentCapability
    status: AgentStatus = AgentStatus.UNKNOWN
    url: Optional[str] = None  # endpoint base URL if applicable
    meta: Dict[str, str] = Field(default_factory=dict)
    registered_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    last_heartbeat: Optional[datetime] = None
    in_flight: int = 0  # current number of active tasks


class AgentMatch(BaseModel):
    """A match result with a score for ranking."""

    agent: AgentInfo
    score: float
    reasons: List[str] = Field(default_factory=list)


class MatchQuery(BaseModel):
    domain: Optional[str] = None
    action: Optional[str] = None
    tags: List[str] = Field(default_factory=list)
    require_healthy: bool = True
    limit: int = 5
