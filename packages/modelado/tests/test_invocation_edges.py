"""Tests for invocation edges (Phase 9.7, Task 7.5)."""

from datetime import datetime
from unittest.mock import MagicMock

import pytest

from modelado.core.invocation_edges import (
    InvocationGraph,
    create_invocation_edges_schema,
)
from modelado.core.model_call_cache import ModelCallCacheFragment


@pytest.fixture
def cache_fragment() -> ModelCallCacheFragment:
    return ModelCallCacheFragment(
        fragment_id="fragment_1",
        model="gpt-4o-mini",
        prompt_hash="prompt_hash",
        seed=42,
        cache_key_id="cache_key_1",
        artifact_id="art_1",
        function_id="fn_1",
        execution_id="exec_1",
        cost_usd=0.001,
        latency_ms=800.0,
        output_hash="out_hash",
        output_length=120,
    )


@pytest.fixture
def mock_connection_pool():
    return MagicMock()


def _build_mock_connection(cursor: MagicMock) -> MagicMock:
    connection = MagicMock()
    connection.__enter__ = MagicMock(return_value=connection)
    connection.__exit__ = MagicMock(return_value=False)
    connection.cursor = MagicMock(return_value=cursor)
    connection.cursor.return_value.__enter__ = MagicMock(return_value=cursor)
    connection.cursor.return_value.__exit__ = MagicMock(return_value=False)
    connection.commit = MagicMock()
    return connection


def test_create_schema(mock_connection_pool):
    cursor = MagicMock()
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    create_invocation_edges_schema(connection)

    # Table + 3 indexes
    assert cursor.execute.call_count == 4
    connection.commit.assert_called_once()


@pytest.mark.asyncio
async def test_add_edge_insert_and_return(mock_connection_pool, cache_fragment):
    created_at = datetime.utcnow()
    row = (
        "edge_1",
        "fn_1",
        "cache_key_1",
        "fragment_1",
        "gpt-4o-mini",
        "prompt_hash",
        42,
        created_at,
    )

    cursor = MagicMock()
    cursor.fetchone.side_effect = [row]
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    graph = InvocationGraph(connection_pool=mock_connection_pool)
    edge = await graph.add_edge("fn_1", cache_fragment)

    assert edge.edge_id == "edge_1"
    assert edge.cache_key_id == "cache_key_1"
    assert edge.fragment_id == "fragment_1"
    assert connection.commit.called


@pytest.mark.asyncio
async def test_add_edge_idempotent(mock_connection_pool, cache_fragment):
    created_at = datetime.utcnow()
    row = (
        "edge_1",
        "fn_1",
        "cache_key_1",
        "fragment_1",
        "gpt-4o-mini",
        "prompt_hash",
        42,
        created_at,
    )

    cursor = MagicMock()
    cursor.fetchone.side_effect = [None, row]
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    graph = InvocationGraph(connection_pool=mock_connection_pool)
    edge = await graph.add_edge("fn_1", cache_fragment)

    assert edge.edge_id == "edge_1"
    # Ensure second fetch was used
    assert cursor.fetchone.call_count == 2


@pytest.mark.asyncio
async def test_get_edges_for_function(mock_connection_pool):
    created_at = datetime.utcnow()
    rows = [
        (
            "edge_1",
            "fn_1",
            "cache_key_1",
            "fragment_1",
            "gpt-4o-mini",
            "prompt_hash",
            42,
            created_at,
        )
    ]

    cursor = MagicMock()
    cursor.fetchall.return_value = rows
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    graph = InvocationGraph(connection_pool=mock_connection_pool)
    edges = await graph.get_edges_for_function("fn_1")

    assert len(edges) == 1
    assert edges[0].function_id == "fn_1"


@pytest.mark.asyncio
async def test_get_edges_for_cache_key(mock_connection_pool):
    created_at = datetime.utcnow()
    rows = [
        (
            "edge_1",
            "fn_1",
            "cache_key_1",
            "fragment_1",
            "gpt-4o-mini",
            "prompt_hash",
            42,
            created_at,
        )
    ]

    cursor = MagicMock()
    cursor.fetchall.return_value = rows
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    graph = InvocationGraph(connection_pool=mock_connection_pool)
    edges = await graph.get_edges_for_cache_key("cache_key_1")

    assert len(edges) == 1
    assert edges[0].cache_key_id == "cache_key_1"


@pytest.mark.asyncio
async def test_get_graph_stats(mock_connection_pool):
    cursor = MagicMock()
    cursor.fetchone.side_effect = [
        (3,),  # total_edges
        (2,),  # functions
        (2,),  # cache_keys
        (2,),  # models
    ]
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    graph = InvocationGraph(connection_pool=mock_connection_pool)
    stats = await graph.get_graph_stats()

    assert stats["total_edges"] == 3
    assert stats["functions"] == 2
    assert stats["cache_keys"] == 2
    assert stats["models"] == 2


@pytest.mark.asyncio
async def test_remove_edge(mock_connection_pool):
    cursor = MagicMock()
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    graph = InvocationGraph(connection_pool=mock_connection_pool)
    await graph.remove_edge("edge_1")

    cursor.execute.assert_any_call("DELETE FROM invocation_edges WHERE edge_id = %s", ("edge_1",))
    assert connection.commit.called


@pytest.mark.asyncio
async def test_clear_edges(mock_connection_pool):
    cursor = MagicMock()
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    graph = InvocationGraph(connection_pool=mock_connection_pool)
    await graph.clear()

    cursor.execute.assert_any_call("DELETE FROM invocation_edges")
    assert connection.commit.called
