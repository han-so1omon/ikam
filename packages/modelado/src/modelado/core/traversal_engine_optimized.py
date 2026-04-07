"""Optimized TraversalEngine with batch graph queries.

Performance Improvements:
- Batch subtree fetching via materialized path prefix scan (eliminates N+1 queries)
- Async-first design compatible with AsyncExecutionLinkGraph
- Pre-built adjacency list for fast neighbor lookups during traversal
- Depth/sibling counts pre-computed for Fisher Information

Expected Performance:
- 100-node subtree fetch: ~1-10ms (vs ~500ms with N+1 queries)
- BFS/DFS traversal: O(N) with single batch query (vs O(N²) with individual queries)
- Memory: O(N) for adjacency list (acceptable for typical graphs <1000 nodes)

Usage:
    from modelado.core.traversal_engine_optimized import OptimizedTraversalEngine
    from modelado.core.execution_links_async import AsyncExecutionLinkGraph
    
    # Initialize with async graph
    graph = AsyncExecutionLinkGraph(connection_pool=async_pool)
    engine = OptimizedTraversalEngine(execution_graph=graph)
    
    # Traverse with batch queries
    progress = await engine.traverse_breadth_first(
        root_execution_id="exec_root",
        config=TraversalConfig(max_workers=10, timing_model=RealisticTimingModel())
    )
    # Single prefix-scan fetches entire subtree, then traversal processes locally
"""

import asyncio
import logging
from collections import defaultdict, deque
from dataclasses import dataclass
from datetime import datetime
from typing import Any, Dict, List, Optional, Set

from modelado.core.execution_links_async import AsyncExecutionLinkGraph, ExecutionLink
from modelado.core.traversal_engine import (
    TraversalConfig,
    TraversalProgress,
    TraversalStepEvent,
    ExecutionStep,
    TraversalOrder,
)

logger = logging.getLogger(__name__)


@dataclass
class GraphAdjacencyList:
    """Pre-built adjacency list for fast traversal lookups.
    
    Performance: O(1) neighbor lookup, O(N) construction.
    """
    
    # Mapping: parent_execution_id → List[ExecutionLink]
    children: Dict[str, List[ExecutionLink]]
    
    # Pre-computed metadata
    depth: Dict[str, int]  # execution_id → depth
    sibling_count: Dict[str, int]  # execution_id → number of siblings
    
    @classmethod
    def from_links(cls, links: List[ExecutionLink]) -> "GraphAdjacencyList":
        """Build adjacency list from execution links.
        
        Args:
            links: All execution links in subtree (from batch query)
            
        Returns:
            GraphAdjacencyList with O(1) lookups
        """
        children: Dict[str, List[ExecutionLink]] = defaultdict(list)
        depth: Dict[str, int] = {}
        sibling_count: Dict[str, int] = {}
        
        # Build children map
        for link in links:
            children[link.caller_execution_id].append(link)
            depth[link.callee_execution_id] = link.depth
        
        # Sort children by invocation_order for deterministic traversal
        for parent_id in children:
            children[parent_id].sort(key=lambda link: link.invocation_order)
            
            # Compute sibling counts
            num_siblings = len(children[parent_id])
            for link in children[parent_id]:
                sibling_count[link.callee_execution_id] = num_siblings - 1
        
        return cls(
            children=dict(children),
            depth=depth,
            sibling_count=sibling_count,
        )


class OptimizedTraversalEngine:
    """Execution traversal engine with batch graph queries.
    
    Optimizations:
    1. Single prefix-scan fetches entire subtree before traversal
    2. Pre-built adjacency list enables O(1) neighbor lookups
    3. Depth/sibling metadata pre-computed for Fisher Information
    4. No blocking DB queries during traversal (all data in memory)
    """
    
    def __init__(
        self,
        execution_graph: AsyncExecutionLinkGraph,
        event_recorder: Optional[Any] = None,
    ):
        """Initialize optimized traversal engine.
        
        Args:
            execution_graph: Async execution link graph for batch queries
            event_recorder: Optional recorder for traversal step events
        """
        self.execution_graph = execution_graph
        self.event_recorder = event_recorder
    
    async def traverse_breadth_first(
        self,
        root_execution_id: str,
        config: TraversalConfig,
    ) -> TraversalProgress:
        """Traverse execution graph breadth-first with batch query optimization.
        
        Performance: O(N) with single batch query + O(N) BFS traversal.
        
        Algorithm:
        1. Fetch entire subtree via materialized path prefix scan (single query)
        2. Build adjacency list in memory (O(N))
        3. BFS traversal using adjacency list (no DB queries, O(N))
        4. Emit traversal step events with pre-computed Fisher Information
        
        Args:
            root_execution_id: Root of subtree to traverse
            config: Traversal configuration
            
        Returns:
            TraversalProgress with completed step counts
        """
        progress = TraversalProgress()
        start_time = datetime.utcnow()
        traversal_id = f"opt_bfs_{root_execution_id[:8]}"
        
        # OPTIMIZATION 1: Batch fetch entire subtree (single prefix-scan query)
        logger.debug(f"Fetching subtree for {root_execution_id} via materialized path prefix scan...")
        subtree_links = await self.execution_graph.get_subtree_by_path(
            root_execution_id=root_execution_id,
            max_depth=config.max_depth,
        )
        logger.debug(f"Fetched {len(subtree_links)} links in subtree")
        
        # OPTIMIZATION 2: Build adjacency list for O(1) lookups
        adjacency = GraphAdjacencyList.from_links(subtree_links)
        
        # OPTIMIZATION 3: BFS traversal using in-memory adjacency list (no DB queries)
        queue = deque([ExecutionStep(
            step_number=0,
            execution_id=root_execution_id,
            function_id="root",
            parent_execution_id=None,
        )])
        
        visited: Set[str] = {root_execution_id}
        step_number = 0
        
        # Process queue (no async needed - all data in memory)
        while queue:
            step = queue.popleft()
            
            # Simulate execution with timing model
            duration = config.timing_model.calculate_duration(step.step_number)
            await asyncio.sleep(duration)
            
            step.status = "completed"
            step.duration_seconds = duration
            progress.completed_steps += 1
            progress.last_step_number = step.step_number
            
            # Emit traversal step event with pre-computed metadata
            if self.event_recorder:
                depth_level = adjacency.depth.get(step.execution_id, 0)
                sibling_count = adjacency.sibling_count.get(step.execution_id, 0)
                
                step_event = TraversalStepEvent(
                    traversal_id=traversal_id,
                    step_number=step.step_number,
                    execution_id=step.execution_id,
                    function_id=step.function_id,
                    parent_execution_id=step.parent_execution_id,
                    duration_seconds=duration,
                    depth_level=depth_level,
                    sibling_count=sibling_count,
                    status="completed",
                )
                
                await self.event_recorder.record_event(step_event)
            
            # Progress callback
            if config.progress_callback:
                await config.progress_callback(progress)
            
            # Enqueue children (O(1) lookup via adjacency list)
            for child_link in adjacency.children.get(step.execution_id, []):
                if child_link.callee_execution_id not in visited:
                    visited.add(child_link.callee_execution_id)
                    step_number += 1
                    
                    child_step = ExecutionStep(
                        step_number=step_number,
                        execution_id=child_link.callee_execution_id,
                        function_id=child_link.callee_function_id,
                        parent_execution_id=step.execution_id,
                    )
                    queue.append(child_step)
        
        # Finalize progress
        progress.elapsed_time_seconds = (datetime.utcnow() - start_time).total_seconds()
        
        if config.progress_callback:
            await config.progress_callback(progress)
        
        return progress
    
    async def traverse_depth_first(
        self,
        root_execution_id: str,
        config: TraversalConfig,
    ) -> TraversalProgress:
        """Traverse execution graph depth-first with batch query optimization.
        
        Performance: O(N) with single batch query + O(N) DFS traversal.
        
        Algorithm:
        1. Fetch entire subtree via materialized path prefix scan (single query)
        2. Build adjacency list in memory
        3. DFS traversal using adjacency list (no DB queries)
        4. Emit traversal step events with pre-computed Fisher Information
        
        Args:
            root_execution_id: Root of subtree to traverse
            config: Traversal configuration
            
        Returns:
            TraversalProgress with completed step counts
        """
        progress = TraversalProgress()
        start_time = datetime.utcnow()
        traversal_id = f"opt_dfs_{root_execution_id[:8]}"
        
        # OPTIMIZATION 1: Batch fetch entire subtree
        logger.debug(f"Fetching subtree for {root_execution_id} via materialized path prefix scan...")
        subtree_links = await self.execution_graph.get_subtree_by_path(
            root_execution_id=root_execution_id,
            max_depth=config.max_depth,
        )
        logger.debug(f"Fetched {len(subtree_links)} links in subtree")
        
        # OPTIMIZATION 2: Build adjacency list
        adjacency = GraphAdjacencyList.from_links(subtree_links)
        
        # OPTIMIZATION 3: DFS traversal using in-memory adjacency list
        visited: Set[str] = set()
        step_number_counter = [0]  # Mutable counter for closure
        
        async def dfs_recursive(execution_id: str, parent_id: Optional[str], depth: int):
            """Recursive DFS helper."""
            if execution_id in visited:
                return
            
            visited.add(execution_id)
            step_number = step_number_counter[0]
            step_number_counter[0] += 1
            
            # Create step
            step = ExecutionStep(
                step_number=step_number,
                execution_id=execution_id,
                function_id="function",  # Would fetch from metadata in production
                parent_execution_id=parent_id,
            )
            
            # Simulate execution
            duration = config.timing_model.calculate_duration(step.step_number)
            await asyncio.sleep(duration)
            
            step.status = "completed"
            step.duration_seconds = duration
            progress.completed_steps += 1
            progress.last_step_number = step.step_number
            
            # Emit event
            if self.event_recorder:
                sibling_count = adjacency.sibling_count.get(execution_id, 0)
                
                step_event = TraversalStepEvent(
                    traversal_id=traversal_id,
                    step_number=step.step_number,
                    execution_id=execution_id,
                    function_id=step.function_id,
                    parent_execution_id=parent_id,
                    duration_seconds=duration,
                    depth_level=depth,
                    sibling_count=sibling_count,
                    status="completed",
                )
                
                await self.event_recorder.record_event(step_event)
            
            # Progress callback
            if config.progress_callback:
                await config.progress_callback(progress)
            
            # Recurse into children
            for child_link in adjacency.children.get(execution_id, []):
                await dfs_recursive(
                    execution_id=child_link.callee_execution_id,
                    parent_id=execution_id,
                    depth=depth + 1,
                )
        
        # Start DFS from root
        await dfs_recursive(root_execution_id, None, 0)
        
        # Finalize progress
        progress.elapsed_time_seconds = (datetime.utcnow() - start_time).total_seconds()
        
        if config.progress_callback:
            await config.progress_callback(progress)
        
        return progress
