import os
from pathlib import Path
from typing import AsyncGenerator, Any

import pytest

try:
    import asyncpg  # type: ignore
except Exception:  # pragma: no cover - handled by skip
    asyncpg = None

from interacciones.registry import AgentCapability, AgentInfo, AgentRegistry, AgentStatus
from interacciones.registry.store_postgres import PostgresRegistryStore


requires_pg = pytest.mark.skipif(
    asyncpg is None or not os.environ.get("TEST_DATABASE_URL"),
    reason="asyncpg or TEST_DATABASE_URL not available",
)


@pytest.fixture
async def pg_pool() -> AsyncGenerator[Any, None]:
    pool = await asyncpg.create_pool(dsn=os.environ.get("TEST_DATABASE_URL"))
    async with pool.acquire() as cx:
        schema_path = Path(__file__).resolve().parent.parent / "schema.sql"
        await cx.execute(schema_path.read_text())
    try:
        yield pool
    finally:
        async with pool.acquire() as cx:
            await cx.execute("DELETE FROM agent_registry WHERE agent_id LIKE 'test-%'")
        await pool.close()


@requires_pg
@pytest.mark.asyncio
async def test_postgres_store_register_and_get(pg_pool):
    store = PostgresRegistryStore(pg_pool)
    reg = AgentRegistry(store)

    a = AgentInfo(
        agent_id="test-econ",
        display_name="Test Econ",
        capabilities=AgentCapability(domains=["economics"], actions=["analyze"], tags=["saas"]),
        status=AgentStatus.HEALTHY,
    )
    await reg.register(a)

    got = await reg.get("test-econ")
    assert got is not None
    assert got.display_name == "Test Econ"
    assert got.status == AgentStatus.HEALTHY

    # update status and heartbeat
    await reg.set_status("test-econ", AgentStatus.DEGRADED)
    upd = await reg.get("test-econ")
    assert upd and upd.status == AgentStatus.DEGRADED

    hb = await reg.heartbeat("test-econ")
    assert hb and hb.last_heartbeat is not None and hb.status in {AgentStatus.DEGRADED, AgentStatus.HEALTHY}


@requires_pg
@pytest.mark.asyncio
async def test_postgres_store_list_and_match(pg_pool):
    store = PostgresRegistryStore(pg_pool)
    reg = AgentRegistry(store)

    await reg.register(
        AgentInfo(
            agent_id="test-econ2",
            capabilities=AgentCapability(domains=["economics"], actions=["predict"], tags=["finance"]),
            status=AgentStatus.HEALTHY,
        )
    )
    await reg.register(
        AgentInfo(
            agent_id="test-story2",
            capabilities=AgentCapability(domains=["story"], actions=["summarize"]),
            status=AgentStatus.HEALTHY,
        )
    )

    agents = await reg.list()
    assert {a.agent_id for a in agents} >= {"test-econ2", "test-story2"}

    matches = await reg.match(domain="economics", action="predict")
    assert matches and matches[0].agent.agent_id == "test-econ2"
