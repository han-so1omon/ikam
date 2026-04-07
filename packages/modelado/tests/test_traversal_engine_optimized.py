"""Optimized traversal engine tests (batch graph queries).

Validates that OptimizedTraversalEngine uses a single batch fetch, emits events,
tracks completed steps, and works for BFS and DFS modes.
"""

import asyncio
from unittest.mock import AsyncMock

import pytest
import pytest_asyncio

from modelado.core.execution_links_async import ExecutionLink
from modelado.core.traversal_engine import TraversalConfig
from modelado.core.traversal_engine_optimized import OptimizedTraversalEngine


class ZeroTimingModel:
    """Timing model that returns zero duration for fast tests."""

    def calculate_duration(self, _step_number: int) -> float:  # pragma: no cover
        return 0.0


@pytest_asyncio.fixture
async def timing_model():
    return ZeroTimingModel()


@pytest_asyncio.fixture
async def sample_links():
    # Tree: root -> child1, child2; child1 -> grand1
    return [
        ExecutionLink(
            link_id="l1",
            caller_execution_id="exec_root",
            callee_execution_id="exec_child1",
            caller_function_id="f_root",
            callee_function_id="f_child",
            invocation_order=0,
            materialized_path="exec_root/exec_child1",
            depth=1,
        ),
        ExecutionLink(
            link_id="l2",
            caller_execution_id="exec_root",
            callee_execution_id="exec_child2",
            caller_function_id="f_root",
            callee_function_id="f_child",
            invocation_order=1,
            materialized_path="exec_root/exec_child2",
            depth=1,
        ),
        ExecutionLink(
            link_id="l3",
            caller_execution_id="exec_child1",
            callee_execution_id="exec_grand1",
            caller_function_id="f_child",
            callee_function_id="f_grand",
            invocation_order=0,
            materialized_path="exec_root/exec_child1/exec_grand1",
            depth=2,
        ),
    ]


@pytest.mark.asyncio
async def test_breadth_first_uses_single_batch_fetch(timing_model, sample_links):
    graph = AsyncMock()
    graph.get_subtree_by_path.return_value = sample_links

    recorder = AsyncMock()
    engine = OptimizedTraversalEngine(execution_graph=graph, event_recorder=recorder)

    config = TraversalConfig(timing_model=timing_model)
    progress = await engine.traverse_breadth_first("exec_root", config)

    # Single batch query
    graph.get_subtree_by_path.assert_awaited_once()

    # Completed steps = root + 3 descendants
    assert progress.completed_steps == 4

    # Events emitted for all nodes
    assert recorder.record_event.await_count == 4


@pytest.mark.asyncio
async def test_depth_first_uses_single_batch_fetch(timing_model, sample_links):
    graph = AsyncMock()
    graph.get_subtree_by_path.return_value = sample_links

    recorder = AsyncMock()
    engine = OptimizedTraversalEngine(execution_graph=graph, event_recorder=recorder)

    config = TraversalConfig(timing_model=timing_model)
    progress = await engine.traverse_depth_first("exec_root", config)

    graph.get_subtree_by_path.assert_awaited_once()
    assert progress.completed_steps == 4
    assert recorder.record_event.await_count == 4
