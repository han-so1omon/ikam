"""Tests for execution linking (Phase 9.7, Task 7.6)."""

from datetime import datetime
from unittest.mock import MagicMock

import pytest

from modelado.core.execution_links import (
    ExecutionLinkGraph,
    create_execution_links_schema,
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

    create_execution_links_schema(connection)

    # Table + 3 indexes
    assert cursor.execute.call_count == 4
    connection.commit.assert_called_once()


@pytest.mark.asyncio
async def test_add_link_insert_and_return(mock_connection_pool):
    created_at = datetime.utcnow()
    row = (
        "link_1",
        "exec_parent_1",
        "exec_child_1",
        "gfn_orchestrator",
        "gfn_analyzer",
        0,
        '{"threshold": 0.85}',
        created_at,
    )

    cursor = MagicMock()
    cursor.fetchone.side_effect = [row]
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    graph = ExecutionLinkGraph(connection_pool=mock_connection_pool)
    link = await graph.add_link(
        caller_execution_id="exec_parent_1",
        callee_execution_id="exec_child_1",
        caller_function_id="gfn_orchestrator",
        callee_function_id="gfn_analyzer",
        invocation_order=0,
        context_snapshot={"threshold": 0.85},
    )

    assert link.link_id == "link_1"
    assert link.caller_execution_id == "exec_parent_1"
    assert link.callee_execution_id == "exec_child_1"
    assert link.invocation_order == 0
    assert connection.commit.called


@pytest.mark.asyncio
async def test_add_link_idempotent(mock_connection_pool):
    created_at = datetime.utcnow()
    row = (
        "link_1",
        "exec_parent_1",
        "exec_child_1",
        "gfn_orchestrator",
        "gfn_analyzer",
        0,
        '{}',
        created_at,
    )

    cursor = MagicMock()
    cursor.fetchone.side_effect = [None, row]
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    graph = ExecutionLinkGraph(connection_pool=mock_connection_pool)
    link = await graph.add_link(
        caller_execution_id="exec_parent_1",
        callee_execution_id="exec_child_1",
        caller_function_id="gfn_orchestrator",
        callee_function_id="gfn_analyzer",
        invocation_order=0,
    )

    assert link.link_id == "link_1"
    # Ensure second fetch was used
    assert cursor.fetchone.call_count == 2


@pytest.mark.asyncio
async def test_get_callee_executions(mock_connection_pool):
    created_at = datetime.utcnow()
    rows = [
        (
            "link_1",
            "exec_parent_1",
            "exec_child_1",
            "gfn_orchestrator",
            "gfn_analyzer",
            0,
            '{}',
            created_at,
        ),
        (
            "link_2",
            "exec_parent_1",
            "exec_child_2",
            "gfn_orchestrator",
            "gfn_validator",
            1,
            '{}',
            created_at,
        ),
    ]

    cursor = MagicMock()
    cursor.fetchall.return_value = rows
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    graph = ExecutionLinkGraph(connection_pool=mock_connection_pool)
    links = await graph.get_callee_executions("exec_parent_1")

    assert len(links) == 2
    assert links[0].callee_execution_id == "exec_child_1"
    assert links[1].callee_execution_id == "exec_child_2"
    assert links[0].invocation_order == 0
    assert links[1].invocation_order == 1


@pytest.mark.asyncio
async def test_get_caller_executions(mock_connection_pool):
    created_at = datetime.utcnow()
    rows = [
        (
            "link_1",
            "exec_parent_1",
            "exec_child_1",
            "gfn_orchestrator",
            "gfn_analyzer",
            0,
            '{}',
            created_at,
        )
    ]

    cursor = MagicMock()
    cursor.fetchall.return_value = rows
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    graph = ExecutionLinkGraph(connection_pool=mock_connection_pool)
    links = await graph.get_caller_executions("exec_child_1")

    assert len(links) == 1
    assert links[0].caller_execution_id == "exec_parent_1"


@pytest.mark.asyncio
async def test_get_function_call_pairs(mock_connection_pool):
    created_at = datetime.utcnow()
    rows = [
        (
            "link_1",
            "exec_parent_1",
            "exec_child_1",
            "gfn_orchestrator",
            "gfn_analyzer",
            0,
            '{}',
            created_at,
        ),
        (
            "link_2",
            "exec_parent_2",
            "exec_child_2",
            "gfn_orchestrator",
            "gfn_analyzer",
            0,
            '{}',
            created_at,
        ),
    ]

    cursor = MagicMock()
    cursor.fetchall.return_value = rows
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    graph = ExecutionLinkGraph(connection_pool=mock_connection_pool)
    links = await graph.get_function_call_pairs("gfn_orchestrator", "gfn_analyzer")

    assert len(links) == 2
    assert all(link.caller_function_id == "gfn_orchestrator" for link in links)
    assert all(link.callee_function_id == "gfn_analyzer" for link in links)


@pytest.mark.asyncio
async def test_get_execution_depth(mock_connection_pool):
    created_at = datetime.utcnow()

    cursor = MagicMock()
    # First call: no parents for root
    # Second call: parent for child
    # Third call: no parents for parent (root)
    cursor.fetchall.side_effect = [
        [],  # Root has no parents
    ]
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    graph = ExecutionLinkGraph(connection_pool=mock_connection_pool)
    depth = await graph.get_execution_depth("exec_root")

    assert depth == 0


@pytest.mark.asyncio
async def test_get_execution_tree(mock_connection_pool):
    created_at = datetime.utcnow()

    cursor = MagicMock()
    # First call: get children of root (full ExecutionLink rows)
    # Second call: get children of child_1 (none)
    # Third call: get children of child_2 (none)
    cursor.fetchall.side_effect = [
        [  # Children of root - full ExecutionLink format
            (
                "link_1",
                "exec_root",
                "exec_child_1",
                "gfn_orchestrator",
                "gfn_analyzer",
                0,
                '{}',
                created_at,
            ),
            (
                "link_2",
                "exec_root",
                "exec_child_2",
                "gfn_orchestrator",
                "gfn_validator",
                1,
                '{}',
                created_at,
            ),
        ],
        [],  # Children of child_1 (none)
        [],  # Children of child_2 (none)
    ]
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    graph = ExecutionLinkGraph(connection_pool=mock_connection_pool)
    tree = await graph.get_execution_tree("exec_root")

    assert tree["execution_id"] == "exec_root"
    assert tree["depth"] == 0
    assert len(tree["children"]) == 2
    assert tree["children"][0]["execution_id"] == "exec_child_1"
    assert tree["children"][1]["execution_id"] == "exec_child_2"


@pytest.mark.asyncio
async def test_get_graph_stats(mock_connection_pool):
    cursor = MagicMock()
    cursor.fetchone.side_effect = [
        (5,),  # total_links
        (3,),  # unique_callers
        (4,),  # unique_callees
        (2,),  # unique_function_pairs
    ]
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    graph = ExecutionLinkGraph(connection_pool=mock_connection_pool)
    stats = await graph.get_graph_stats()

    assert stats["total_links"] == 5
    assert stats["unique_callers"] == 3
    assert stats["unique_callees"] == 4
    assert stats["unique_function_pairs"] == 2


@pytest.mark.asyncio
async def test_remove_link(mock_connection_pool):
    cursor = MagicMock()
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    graph = ExecutionLinkGraph(connection_pool=mock_connection_pool)
    await graph.remove_link("link_1")

    cursor.execute.assert_any_call("DELETE FROM execution_links WHERE link_id = %s", ("link_1",))
    assert connection.commit.called


@pytest.mark.asyncio
async def test_clear_links(mock_connection_pool):
    cursor = MagicMock()
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    graph = ExecutionLinkGraph(connection_pool=mock_connection_pool)
    await graph.clear()

    cursor.execute.assert_any_call("DELETE FROM execution_links")
    assert connection.commit.called
