import pytest

from interacciones.registry import (
    AgentCapability,
    AgentInfo,
    AgentRegistry,
    AgentStatus,
)


@pytest.mark.asyncio
async def test_register_and_get_and_list_inmemory():
    reg = AgentRegistry()

    a1 = AgentInfo(
        agent_id="econ-modeler",
        display_name="Economic Modeler",
        capabilities=AgentCapability(
            domains=["economics"], actions=["analyze", "predict"], tags=["saas", "finance"], max_concurrency=2
        ),
        status=AgentStatus.HEALTHY,
    )

    await reg.register(a1)

    # get
    got = await reg.get("econ-modeler")
    assert got is not None
    assert got.agent_id == "econ-modeler"

    # list
    agents = await reg.list()
    assert len(agents) == 1
    assert agents[0].display_name == "Economic Modeler"


@pytest.mark.asyncio
async def test_match_simple():
    reg = AgentRegistry()
    await reg.register(
        AgentInfo(
            agent_id="econ",
            capabilities=AgentCapability(domains=["economics"], actions=["analyze"], tags=["saas"]),
            status=AgentStatus.HEALTHY,
        )
    )
    await reg.register(
        AgentInfo(
            agent_id="story",
            capabilities=AgentCapability(domains=["story"], actions=["summarize"], tags=["pitch"]),
            status=AgentStatus.HEALTHY,
        )
    )

    matches = await reg.match(domain="economics", action="analyze")
    assert len(matches) >= 1
    assert matches[0].agent.agent_id == "econ"
    assert matches[0].score > 0


@pytest.mark.asyncio
async def test_capacity_penalty_and_status_filtering():
    reg = AgentRegistry()
    await reg.register(
        AgentInfo(
            agent_id="healthy-near-capacity",
            capabilities=AgentCapability(domains=["economics"], actions=["analyze"], max_concurrency=2),
            status=AgentStatus.HEALTHY,
            in_flight=2,
        )
    )
    await reg.register(
        AgentInfo(
            agent_id="degraded",
            capabilities=AgentCapability(domains=["economics"], actions=["analyze"], max_concurrency=10),
            status=AgentStatus.DEGRADED,
        )
    )
    await reg.register(
        AgentInfo(
            agent_id="unhealthy",
            capabilities=AgentCapability(domains=["economics"], actions=["analyze"]),
            status=AgentStatus.UNHEALTHY,
        )
    )

    matches = await reg.match(domain="economics", action="analyze", require_healthy=True)
    ids = [m.agent.agent_id for m in matches]
    assert "unhealthy" not in ids
    assert any(m.agent.agent_id == "degraded" for m in matches)
    # ensure degraded gets penalty and doesn't outrank healthy near capacity too much
    scores = {m.agent.agent_id: m.score for m in matches}
    assert scores["degraded"] <= scores["healthy-near-capacity"] + 5


@pytest.mark.asyncio
async def test_heartbeat_and_status_update():
    reg = AgentRegistry()
    await reg.register(
        AgentInfo(
            agent_id="hb",
            capabilities=AgentCapability(domains=["economics"], actions=["analyze"]),
        )
    )
    before = await reg.get("hb")
    assert before is not None and before.last_heartbeat is None and before.status == AgentStatus.UNKNOWN

    after = await reg.heartbeat("hb")
    assert after is not None and after.last_heartbeat is not None and after.status == AgentStatus.HEALTHY

    upd = await reg.set_status("hb", AgentStatus.DRAINING)
    assert upd is not None and upd.status == AgentStatus.DRAINING
