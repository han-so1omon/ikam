"""Test agent capacity tracking and in_flight gating.

Validates that:
- Agents can update in_flight counts via heartbeat or registration
- Matchers apply capacity penalties based on in_flight / max_concurrency ratio
- Agents at capacity are deprioritized but not excluded
"""
import pytest
from interacciones.registry import (
    AgentCapability,
    AgentInfo,
    AgentRegistry,
    AgentStatus,
)


@pytest.mark.asyncio
async def test_in_flight_counter_via_heartbeat():
    """Verify in_flight count can be updated via agent registration or programmatically."""
    reg = AgentRegistry()
    
    # Register agent with initial in_flight
    await reg.register(
        AgentInfo(
            agent_id="worker-1",
            capabilities=AgentCapability(domains=["economics"], actions=["analyze"], max_concurrency=5),
            status=AgentStatus.HEALTHY,
            in_flight=0,
        )
    )
    
    agent = await reg.get("worker-1")
    assert agent is not None
    assert agent.in_flight == 0
    
    # Increment in_flight (simulates task assignment)
    updated = await reg.increment_in_flight("worker-1", delta=1)
    assert updated is not None
    assert updated.in_flight == 1
    
    # Increment again
    updated = await reg.increment_in_flight("worker-1", delta=2)
    assert updated is not None
    assert updated.in_flight == 3
    
    # Decrement (task completion)
    updated = await reg.increment_in_flight("worker-1", delta=-1)
    assert updated is not None
    assert updated.in_flight == 2
    
    # Ensure floor at zero
    await reg.increment_in_flight("worker-1", delta=-100)
    final = await reg.get("worker-1")
    assert final is not None
    assert final.in_flight == 0


@pytest.mark.asyncio
async def test_capacity_penalty_in_matching():
    """Verify agents near capacity receive score penalties in matching."""
    reg = AgentRegistry()
    
    # Agent with high capacity utilization
    await reg.register(
        AgentInfo(
            agent_id="saturated",
            capabilities=AgentCapability(domains=["economics"], actions=["analyze"], max_concurrency=10),
            status=AgentStatus.HEALTHY,
            in_flight=9,  # 90% utilization
        )
    )
    
    # Agent with low capacity utilization
    await reg.register(
        AgentInfo(
            agent_id="available",
            capabilities=AgentCapability(domains=["economics"], actions=["analyze"], max_concurrency=10),
            status=AgentStatus.HEALTHY,
            in_flight=1,  # 10% utilization
        )
    )
    
    # Match for economics + analyze
    matches = await reg.match(domain="economics", action="analyze", require_healthy=True)
    
    # Both should be returned, but "available" should have higher score
    ids = [m.agent.agent_id for m in matches]
    assert "saturated" in ids
    assert "available" in ids
    
    scores = {m.agent.agent_id: m.score for m in matches}
    assert scores["available"] > scores["saturated"], "Agent with lower capacity utilization should score higher"


@pytest.mark.asyncio
async def test_capacity_gating_at_full_capacity():
    """Verify agents at 100% capacity are still matched but heavily penalized."""
    reg = AgentRegistry()
    
    # Agent at full capacity
    await reg.register(
        AgentInfo(
            agent_id="maxed-out",
            capabilities=AgentCapability(domains=["economics"], actions=["analyze"], max_concurrency=5),
            status=AgentStatus.HEALTHY,
            in_flight=5,  # 100% capacity
        )
    )
    
    # Agent with capacity
    await reg.register(
        AgentInfo(
            agent_id="has-room",
            capabilities=AgentCapability(domains=["economics"], actions=["analyze"], max_concurrency=5),
            status=AgentStatus.HEALTHY,
            in_flight=2,  # 40% capacity
        )
    )
    
    matches = await reg.match(domain="economics", action="analyze")
    
    # Both agents should be returned (no hard exclusion)
    ids = [m.agent.agent_id for m in matches]
    assert len(ids) == 2
    assert "maxed-out" in ids
    assert "has-room" in ids
    
    # Agent with capacity should rank first
    assert matches[0].agent.agent_id == "has-room"
    
    scores = {m.agent.agent_id: m.score for m in matches}
    # The agent at full capacity should have a lower score due to capacity penalty
    # Formula: penalty = 0.5 * max(0, in_flight - (max_c - 2))
    # For maxed-out: penalty = 0.5 * (5 - 3) = 1.0
    # For has-room: penalty = 0 (2 < 3)
    assert scores["has-room"] > scores["maxed-out"], "Agent with capacity should have higher score than agent at capacity"


@pytest.mark.asyncio
async def test_status_transitions_affect_matching():
    """Verify agent status transitions (healthy → degraded → unhealthy) affect matching results."""
    reg = AgentRegistry()
    
    await reg.register(
        AgentInfo(
            agent_id="status-agent",
            capabilities=AgentCapability(domains=["economics"], actions=["analyze"]),
            status=AgentStatus.HEALTHY,
        )
    )
    
    # Healthy: should be matched
    matches = await reg.match(domain="economics", action="analyze", require_healthy=True)
    assert len(matches) == 1
    assert matches[0].agent.agent_id == "status-agent"
    
    # Degrade status
    await reg.set_status("status-agent", AgentStatus.DEGRADED)
    
    # Degraded: still matched with require_healthy=True (degraded is acceptable)
    matches = await reg.match(domain="economics", action="analyze", require_healthy=True)
    assert len(matches) == 1
    # But score should be penalized
    degraded_score = matches[0].score
    
    # Mark unhealthy
    await reg.set_status("status-agent", AgentStatus.UNHEALTHY)
    
    # Unhealthy: excluded when require_healthy=True
    matches = await reg.match(domain="economics", action="analyze", require_healthy=True)
    assert len(matches) == 0
    
    # But included when require_healthy=False
    matches = await reg.match(domain="economics", action="analyze", require_healthy=False)
    assert len(matches) == 1
    unhealthy_score = matches[0].score
    
    # Unhealthy should have lower score than degraded
    assert unhealthy_score < degraded_score


@pytest.mark.asyncio
async def test_heartbeat_updates_and_auto_transition():
    """Verify heartbeat updates last_heartbeat and auto-transitions UNKNOWN → HEALTHY."""
    reg = AgentRegistry()
    
    # Register agent with UNKNOWN status (default)
    await reg.register(
        AgentInfo(
            agent_id="new-agent",
            capabilities=AgentCapability(domains=["economics"], actions=["analyze"]),
        )
    )
    
    agent = await reg.get("new-agent")
    assert agent is not None
    assert agent.status == AgentStatus.UNKNOWN
    assert agent.last_heartbeat is None
    
    # First heartbeat should transition to HEALTHY
    updated = await reg.heartbeat("new-agent")
    assert updated is not None
    assert updated.status == AgentStatus.HEALTHY
    assert updated.last_heartbeat is not None
    
    # Subsequent heartbeats preserve HEALTHY status
    updated2 = await reg.heartbeat("new-agent")
    assert updated2 is not None
    assert updated2.status == AgentStatus.HEALTHY
    assert updated2.last_heartbeat >= updated.last_heartbeat
