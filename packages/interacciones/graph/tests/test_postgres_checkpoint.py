"""Tests for PostgresCheckpoint using asyncpg.

These tests require a PostgreSQL database and asyncpg installed.
Skip if postgres is not available.
"""
import os
import pytest

try:
    import asyncpg
    ASYNCPG_AVAILABLE = True
except ImportError:
    ASYNCPG_AVAILABLE = False

from interacciones.graph import PostgresCheckpoint, AgentGraph, FunctionNode, HumanNode, append_reducer


# Skip all tests if asyncpg not available or no DB configured
pytestmark = pytest.mark.skipif(
    not ASYNCPG_AVAILABLE or not os.getenv("TEST_DATABASE_URL"),
    reason="asyncpg or TEST_DATABASE_URL not available"
)


@pytest.fixture
async def db_pool():
    """Create asyncpg pool for testing."""
    database_url = os.getenv("TEST_DATABASE_URL", "postgresql://user:pass@localhost:5432/app")
    pool = await asyncpg.create_pool(database_url, min_size=1, max_size=2)
    
    # Ensure schema exists
    async with pool.acquire() as conn:
        await conn.execute("""
            CREATE TABLE IF NOT EXISTS graph_checkpoints (
                execution_id TEXT PRIMARY KEY,
                state JSONB NOT NULL,
                next_node TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
        """)
    
    yield pool
    
    # Cleanup
    async with pool.acquire() as conn:
        await conn.execute("DELETE FROM graph_checkpoints WHERE execution_id LIKE 'test-%'")
    await pool.close()


async def test_postgres_checkpoint_save_and_load(db_pool):
    cp = PostgresCheckpoint(db_pool)
    
    state = {"x": 1, "messages": ["hello"]}
    await cp.save("test-exec-1", state, "next_node")
    
    loaded = await cp.load("test-exec-1")
    assert loaded is not None
    loaded_state, next_node = loaded
    assert loaded_state == state
    assert next_node == "next_node"


async def test_postgres_checkpoint_upsert(db_pool):
    cp = PostgresCheckpoint(db_pool)
    
    # First save
    await cp.save("test-exec-2", {"x": 1}, "node1")
    
    # Update
    await cp.save("test-exec-2", {"x": 2, "y": 3}, "node2")
    
    loaded = await cp.load("test-exec-2")
    assert loaded is not None
    loaded_state, next_node = loaded
    assert loaded_state == {"x": 2, "y": 3}
    assert next_node == "node2"


async def test_postgres_checkpoint_load_missing(db_pool):
    cp = PostgresCheckpoint(db_pool)
    
    loaded = await cp.load("test-exec-nonexistent")
    assert loaded is None


async def test_postgres_checkpoint_with_graph(db_pool):
    """Integration test: use PostgresCheckpoint with AgentGraph."""
    def parse(state):
        return {"parsed": True, "messages": ["parse"]}
    
    def finalize(state):
        return {"final": True, "messages": ["final"]}
    
    cp = PostgresCheckpoint(db_pool)
    graph = AgentGraph(
        reducers={"messages": append_reducer},
        checkpointer=cp,
        interrupt_before=["approval"],
    )
    
    graph.add_node("parse", FunctionNode(parse, "parse"))
    graph.add_node("approval", HumanNode("Approve?", "approval"))
    graph.add_node("finalize", FunctionNode(finalize, "finalize"))
    
    graph.add_edge("parse", "approval")
    graph.add_edge("approval", "finalize")
    
    # Execute and pause
    state = await graph.execute({}, start="parse", execution_id="test-exec-3")
    assert state["parsed"] is True
    
    # Verify checkpoint in DB
    loaded = await cp.load("test-exec-3")
    assert loaded is not None
    _, next_node = loaded
    assert next_node == "approval"
    
    # Resume
    resumed = await graph.resume("test-exec-3", updated_state={"approved": True})
    assert resumed["final"] is True
    assert resumed["approved"] is True
