"""Factory helpers to select traversal engine (optimized vs standard).

- If an AsyncExecutionLinkGraph is provided and optimized=True, returns OptimizedTraversalEngine
- Otherwise returns the standard TraversalEngine
"""

from __future__ import annotations

from typing import Any

from modelado.core.execution_links import ExecutionLinkGraph
from modelado.core.execution_links_async import AsyncExecutionLinkGraph
from modelado.core.traversal_engine import TraversalEngine
from modelado.core.traversal_engine_optimized import OptimizedTraversalEngine


def create_traversal_engine(
    execution_graph: Any,
    *,
    event_recorder: Any = None,
    timing_model: Any = None,
    optimized: bool = False,
):
    """Return the appropriate traversal engine for the given graph.

    Args:
        execution_graph: Graph instance (sync or async)
        event_recorder: Optional recorder for traversal events
        timing_model: Optional timing model for traversal
        optimized: If True and graph is async, use OptimizedTraversalEngine
    """
    if optimized and isinstance(execution_graph, AsyncExecutionLinkGraph):
        return OptimizedTraversalEngine(
            execution_graph=execution_graph,
            event_recorder=event_recorder,
        )
    # Fallback to standard engine (accepts timing_model)
    return TraversalEngine(
        execution_graph,
        timing_model=timing_model,
        event_recorder=event_recorder,
    )
