import asyncio
from unittest.mock import AsyncMock, MagicMock

import pytest

from modelado.core.execution_links import ExecutionLinkGraph
from modelado.core.execution_links_async import AsyncExecutionLinkGraph
from modelado.core.traversal_engine import TraversalEngine
from modelado.core.traversal_engine_optimized import OptimizedTraversalEngine
from modelado.core.traversal_engine_factory import create_traversal_engine


class DummyTimingModel:
    def calculate_duration(self, _step_number: int) -> float:
        return 0.0


def test_factory_returns_standard_for_sync_graph():
    graph = MagicMock(spec=ExecutionLinkGraph)
    engine = create_traversal_engine(graph, timing_model=DummyTimingModel())
    assert isinstance(engine, TraversalEngine)


def test_factory_returns_optimized_for_async_graph_when_enabled():
    graph = MagicMock(spec=AsyncExecutionLinkGraph)
    engine = create_traversal_engine(graph, optimized=True)
    assert isinstance(engine, OptimizedTraversalEngine)


def test_factory_falls_back_to_standard_when_not_optimized():
    graph = MagicMock(spec=AsyncExecutionLinkGraph)
    engine = create_traversal_engine(graph, optimized=False)
    assert isinstance(engine, TraversalEngine)
