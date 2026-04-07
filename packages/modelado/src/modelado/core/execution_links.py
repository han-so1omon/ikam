"""Execution linking: caller→callee relationships for function invocations.

Responsibilities:
- Link parent function executions to child function executions
- Enable call graph traversal for provenance analysis
- Track execution context propagation (parameters, artifacts, costs)
- Support Fisher Information uplift validation (I_linked > I_flat)

Schema:
    execution_links(
        link_id TEXT PRIMARY KEY,
        caller_execution_id TEXT NOT NULL,
        callee_execution_id TEXT NOT NULL,
        caller_function_id TEXT NOT NULL,
        callee_function_id TEXT NOT NULL,
        invocation_order INTEGER NOT NULL,  -- 0-indexed order within caller
        context_snapshot JSONB,  -- Parameters passed from caller to callee
        created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
        UNIQUE (caller_execution_id, invocation_order)
    )

Indexes:
- idx_execution_links_caller (caller_execution_id, invocation_order)
- idx_execution_links_callee (callee_execution_id, created_at DESC)
- idx_execution_links_function_pair (caller_function_id, callee_function_id)

Fisher Information Properties:
- I_linked(θ) ≥ I_caller(θ) + I_callee(θ)
- Additional information from call graph structure
- Context snapshot preserves parameter binding for reconstruction

Usage:
    from modelado.core.execution_links import ExecutionLinkGraph, ExecutionLink
    
    # Initialize graph
    graph = ExecutionLinkGraph(connection_pool=db_pool)
    
    # Record caller→callee link
    link = await graph.add_link(
        caller_execution_id="exec_parent_123",
        callee_execution_id="exec_child_456",
        caller_function_id="gfn_orchestrator",
        callee_function_id="gfn_analyzer",
        invocation_order=0,
        context_snapshot={"input_data": "revenue_model", "threshold": 0.85}
    )
    
    # Query call graph
    children = await graph.get_callee_executions(caller_execution_id="exec_parent_123")
    parents = await graph.get_caller_executions(callee_execution_id="exec_child_456")
    
    # Compute FI uplift
    fi_uplift = await graph.compute_fi_uplift(execution_id="exec_parent_123")
"""

from __future__ import annotations

import logging
from datetime import datetime
from typing import Any, Dict, List, Optional, Set
from uuid import uuid4

import psycopg
from pydantic import BaseModel, ConfigDict, Field

from modelado.core.invocation_policy import (
    InvocationPolicy,
    PolicyEnforcer,
    PolicyViolation,
    DEFAULT_POLICY,
)

logger = logging.getLogger(__name__)


class ExecutionLink(BaseModel):
    """A single caller→callee execution link."""
    
    model_config = ConfigDict(arbitrary_types_allowed=True)
    
    link_id: str = Field(default_factory=lambda: str(uuid4()))
    caller_execution_id: str = Field(..., description="Parent execution ID")
    callee_execution_id: str = Field(..., description="Child execution ID")
    caller_function_id: str = Field(..., description="Parent function ID")
    callee_function_id: str = Field(..., description="Child function ID")
    invocation_order: int = Field(..., description="0-indexed order within caller (deterministic)")
    context_snapshot: Dict[str, Any] = Field(
        default_factory=dict,
        description="Parameters/context passed from caller to callee"
    )
    created_at: datetime = Field(default_factory=datetime.utcnow)


class ExecutionLinkGraph:
    """Manager for execution links (caller→callee relationships)."""
    
    def __init__(
        self,
        connection_pool: Optional[psycopg.ConnectionPool] = None,
        policy: Optional[InvocationPolicy] = None,
    ):
        # Backwards compatibility: older tests passed a sqlite file path here.
        # The current implementation supports either Postgres (via ConnectionPool)
        # or a pure in-memory graph.
        self.db_path: Optional[str] = None
        if isinstance(connection_pool, str):
            self.db_path = connection_pool
            self.connection_pool = None
        else:
            self.connection_pool = connection_pool
        self.policy = policy or DEFAULT_POLICY
        self.policy_enforcer = PolicyEnforcer(self.policy)
        # Track elapsed time + reset counters from the moment the graph is created.
        # Older tests assume this is active without an explicit start call.
        self.policy_enforcer.start_tracking()
        # In-memory representation used by tests:
        # caller -> {callee -> invocation_order}
        self.graph: Dict[str, Dict[str, int]] = {}
        self._memory_links_by_key: Dict[tuple[str, int], ExecutionLink] = {}

        logger.info(
            "ExecutionLinkGraph initialized "
            f"(mode={'postgres' if self.connection_pool else 'memory'}, "
            f"max_depth={self.policy.max_depth}, max_fan_out={self.policy.max_fan_out})"
        )

    # ---------------------------------------------------------------------
    # In-memory graph API (tests)
    # ---------------------------------------------------------------------
    def add_invocation(
        self,
        caller_id: str,
        callee_id: str,
        invocation_order: int,
        context: Optional[Dict[str, Any]] = None,
    ) -> None:
        self.graph.setdefault(caller_id, {})[callee_id] = invocation_order
        self.graph.setdefault(callee_id, {})

    def compute_depths(self) -> Dict[str, int]:
        """Compute minimal depth-from-root for each node."""
        if not self.graph:
            return {}

        callees: Set[str] = set()
        for caller, children in self.graph.items():
            callees.update(children.keys())

        roots = [n for n in self.graph.keys() if n not in callees]
        if not roots:
            # Cycle-only or fully connected; pick arbitrary roots for determinism.
            roots = sorted(self.graph.keys())[:1]

        depths: Dict[str, int] = {}
        queue: List[tuple[str, int]] = [(r, 0) for r in roots]
        for r in roots:
            depths[r] = 0

        while queue:
            node, d = queue.pop(0)
            for child in self.graph.get(node, {}).keys():
                nd = d + 1
                if child not in depths or nd < depths[child]:
                    depths[child] = nd
                    queue.append((child, nd))

        # Ensure all nodes exist in the mapping.
        for node in self.graph.keys():
            depths.setdefault(node, 0)

        return depths
    
    async def add_link(
        self,
        caller_execution_id: str,
        callee_execution_id: str,
        caller_function_id: str,
        callee_function_id: str,
        invocation_order: int,
        context_snapshot: Optional[Dict[str, Any]] = None,
        cost: float = 0.0,
        enforce_policy: bool = True,
    ) -> ExecutionLink:
        """Add a caller→callee execution link with policy enforcement.
        
        Args:
            caller_execution_id: Parent execution ID
            callee_execution_id: Child execution ID
            caller_function_id: Parent function ID
            callee_function_id: Child function ID
            invocation_order: 0-indexed order within caller
            context_snapshot: Parameters passed from caller to callee
            cost: Cost of callee execution in USD
            enforce_policy: Whether to enforce policy limits (default: True)
            
        Returns:
            ExecutionLink if successful
            
        Raises:
            RuntimeError: If policy violation detected and enforce_policy=True
        """
        import json
        
        # Check policy limits before adding
        if enforce_policy:
            # Get current depth and fan-out
            callee_depth = await self.get_execution_depth(caller_execution_id) + 1
            current_fan_out = len(await self.get_callee_executions(caller_execution_id))
            
            # Check for violations
            violation = self.policy_enforcer.check_before_add_link(
                caller_execution_id=caller_execution_id,
                callee_depth=callee_depth,
                current_fan_out=current_fan_out,
                new_callee_cost=cost,
            )
            
            if violation:
                logger.error(f"Policy violation: {violation.message}")
                violation_type = getattr(violation, "violation_type", None)
                if violation_type == "max_depth":
                    raise RuntimeError(f"depth limit exceeded: {violation.message}")
                if violation_type == "max_fan_out":
                    raise RuntimeError(f"fan-out limit exceeded: {violation.message}")
                if violation_type == "max_cost":
                    raise RuntimeError(f"cost budget exceeded: {violation.message}")
                if violation_type == "max_executions":
                    raise RuntimeError(f"execution count exceeded: {violation.message}")
                if violation_type == "max_time":
                    raise RuntimeError(f"execution time exceeded: {violation.message}")
                raise RuntimeError(f"policy violation: {violation.message}")
            
            # Check for cycles (if not allowed)
            if not self.policy.allow_cycles:
                # Build current graph
                execution_graph = await self._build_adjacency_list()
                cycle_violation = self.policy_enforcer.check_cycle(
                    caller_execution_id=caller_execution_id,
                    callee_execution_id=callee_execution_id,
                    execution_graph=execution_graph,
                )
                
                if cycle_violation:
                    logger.error(f"cycle detected: {cycle_violation.message}")
                    raise RuntimeError(f"cycle detected: {cycle_violation.message}")

        # Record the execution (and cost) for stats, even when policy enforcement
        # is disabled for a specific link.
        self.policy_enforcer.record_execution(cost=cost)

        link = ExecutionLink(
            caller_execution_id=caller_execution_id,
            callee_execution_id=callee_execution_id,
            caller_function_id=caller_function_id,
            callee_function_id=callee_function_id,
            invocation_order=invocation_order,
            context_snapshot=context_snapshot or {},
        )

        # In-memory mode: persist only to the in-process graph.
        if self.connection_pool is None:
            self.graph.setdefault(caller_execution_id, {})[callee_execution_id] = invocation_order
            self.graph.setdefault(callee_execution_id, {})
            self._memory_links_by_key[(caller_execution_id, invocation_order)] = link
            return link

        with self.connection_pool.connection() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    """
                    INSERT INTO execution_links (
                        link_id, caller_execution_id, callee_execution_id,
                        caller_function_id, callee_function_id, invocation_order,
                        context_snapshot, created_at
                    ) VALUES (%s, %s, %s, %s, %s, %s, %s, %s)
                    ON CONFLICT (caller_execution_id, invocation_order) DO NOTHING
                    RETURNING link_id, caller_execution_id, callee_execution_id,
                              caller_function_id, callee_function_id, invocation_order,
                              context_snapshot, created_at
                    """,
                    (
                        link.link_id,
                        link.caller_execution_id,
                        link.callee_execution_id,
                        link.caller_function_id,
                        link.callee_function_id,
                        link.invocation_order,
                        json.dumps(link.context_snapshot),
                        link.created_at,
                    ),
                )
                row = cur.fetchone()
                
                # If link already exists, fetch it for deterministic return
                if row is None:
                    cur.execute(
                        """
                        SELECT link_id, caller_execution_id, callee_execution_id,
                               caller_function_id, callee_function_id, invocation_order,
                               context_snapshot, created_at
                        FROM execution_links
                        WHERE caller_execution_id = %s AND invocation_order = %s
                        """,
                        (caller_execution_id, invocation_order),
                    )
                    row = cur.fetchone()
            
            cx.commit()
        
        if row is None:
            raise RuntimeError("Execution link could not be created or fetched")
        
        return self._row_to_link(row)
    
    async def get_callee_executions(
        self,
        caller_execution_id: str,
    ) -> List[ExecutionLink]:
        """Get all callee executions for a caller (ordered by invocation_order)."""
        if self.connection_pool is None:
            links = [
                link
                for (caller_id, _), link in self._memory_links_by_key.items()
                if caller_id == caller_execution_id
            ]
            return sorted(links, key=lambda l: l.invocation_order)

        with self.connection_pool.connection() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    """
                    SELECT link_id, caller_execution_id, callee_execution_id,
                           caller_function_id, callee_function_id, invocation_order,
                           context_snapshot, created_at
                    FROM execution_links
                    WHERE caller_execution_id = %s
                    ORDER BY invocation_order ASC
                    """,
                    (caller_execution_id,),
                )
                rows = cur.fetchall()
        
        return [self._row_to_link(r) for r in rows]
    
    async def get_caller_executions(
        self,
        callee_execution_id: str,
    ) -> List[ExecutionLink]:
        """Get all caller executions for a callee (ordered by created_at)."""
        if self.connection_pool is None:
            links = [
                link
                for link in self._memory_links_by_key.values()
                if link.callee_execution_id == callee_execution_id
            ]
            return sorted(links, key=lambda l: l.created_at)

        with self.connection_pool.connection() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    """
                    SELECT link_id, caller_execution_id, callee_execution_id,
                           caller_function_id, callee_function_id, invocation_order,
                           context_snapshot, created_at
                    FROM execution_links
                    WHERE callee_execution_id = %s
                    ORDER BY created_at ASC
                    """,
                    (callee_execution_id,),
                )
                rows = cur.fetchall()
        
        return [self._row_to_link(r) for r in rows]
    
    async def get_function_call_pairs(
        self,
        caller_function_id: str,
        callee_function_id: str,
    ) -> List[ExecutionLink]:
        """Get all executions where function A called function B."""
        with self.connection_pool.connection() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    """
                    SELECT link_id, caller_execution_id, callee_execution_id,
                           caller_function_id, callee_function_id, invocation_order,
                           context_snapshot, created_at
                    FROM execution_links
                    WHERE caller_function_id = %s AND callee_function_id = %s
                    ORDER BY created_at DESC
                    """,
                    (caller_function_id, callee_function_id),
                )
                rows = cur.fetchall()
        
        return [self._row_to_link(r) for r in rows]
    
    async def get_execution_depth(self, execution_id: str) -> int:
        """Calculate execution depth (0 for root, 1 for direct child, etc.)."""
        if self.connection_pool is None:
            depths = self.compute_depths()
            return depths.get(execution_id, 0)

        depth = 0
        current_id = execution_id
        visited = {execution_id}  # Cycle detection
        
        while True:
            parents = await self.get_caller_executions(current_id)
            if not parents:
                break
            
            # Take first parent (assumes single caller per execution)
            parent = parents[0]
            if parent.caller_execution_id in visited:
                # Cycle detected
                logger.warning(f"Cycle detected in execution chain: {execution_id}")
                break
            
            visited.add(parent.caller_execution_id)
            current_id = parent.caller_execution_id
            depth += 1
        
        return depth
    
    async def get_execution_tree(
        self,
        root_execution_id: str,
        max_depth: int = 10,
    ) -> Dict[str, Any]:
        """Get full execution tree rooted at given execution (BFS traversal)."""
        tree = {
            "execution_id": root_execution_id,
            "depth": 0,
            "children": [],
        }
        
        async def build_tree(node: Dict[str, Any], current_depth: int):
            if current_depth >= max_depth:
                return
            
            children = await self.get_callee_executions(node["execution_id"])
            for child_link in children:
                child_node = {
                    "execution_id": child_link.callee_execution_id,
                    "function_id": child_link.callee_function_id,
                    "invocation_order": child_link.invocation_order,
                    "depth": current_depth + 1,
                    "children": [],
                }
                node["children"].append(child_node)
                await build_tree(child_node, current_depth + 1)
        
        await build_tree(tree, 0)
        return tree
    
    async def remove_link(self, link_id: str) -> None:
        """Remove a specific execution link."""
        with self.connection_pool.connection() as cx:
            with cx.cursor() as cur:
                cur.execute("DELETE FROM execution_links WHERE link_id = %s", (link_id,))
            cx.commit()
    
    async def clear(self) -> None:
        """Delete all execution links (test utility)."""
        with self.connection_pool.connection() as cx:
            with cx.cursor() as cur:
                cur.execute("DELETE FROM execution_links")
            cx.commit()
    
    async def get_graph_stats(self) -> Dict[str, int]:
        """Return basic execution graph statistics."""
        with self.connection_pool.connection() as cx:
            with cx.cursor() as cur:
                cur.execute("SELECT COUNT(*) FROM execution_links")
                total_links = cur.fetchone()[0]
                
                cur.execute("SELECT COUNT(DISTINCT caller_execution_id) FROM execution_links")
                unique_callers = cur.fetchone()[0]
                
                cur.execute("SELECT COUNT(DISTINCT callee_execution_id) FROM execution_links")
                unique_callees = cur.fetchone()[0]
                
                cur.execute(
                    "SELECT COUNT(DISTINCT caller_function_id || '→' || callee_function_id) "
                    "FROM execution_links"
                )
                unique_function_pairs = cur.fetchone()[0]
        
        return {
            "total_links": total_links,
            "unique_callers": unique_callers,
            "unique_callees": unique_callees,
            "unique_function_pairs": unique_function_pairs,
        }
    
    async def _build_adjacency_list(self) -> Dict[str, List[str]]:
        """Build adjacency list representation of execution graph for cycle detection.
        
        Returns:
            Dict mapping caller_execution_id → list of callee_execution_ids
        """
        if self.connection_pool is None:
            return {caller_id: list(children.keys()) for caller_id, children in self.graph.items()}

        graph: Dict[str, List[str]] = {}
        
        with self.connection_pool.connection() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    "SELECT caller_execution_id, callee_execution_id FROM execution_links"
                )
                rows = cur.fetchall()
        
        for caller_id, callee_id in rows:
            if caller_id not in graph:
                graph[caller_id] = []
            graph[caller_id].append(callee_id)
        
        return graph
    
    def get_policy_statistics(self) -> Dict[str, Any]:
        """Get current policy enforcement statistics.
        
        Returns:
            Dictionary with policy metrics and utilization percentages
        """
        stats = self.policy_enforcer.get_statistics()

        # Backwards-compatible flattened keys expected by policy integration tests.
        # Keep the nested shape (`policy`, `utilization`) for newer callers.
        stats["max_depth"] = stats.get("policy", {}).get("max_depth")
        stats["max_fan_out"] = stats.get("policy", {}).get("max_fan_out")
        stats["cost_utilization_percent"] = stats.get("utilization", {}).get("cost_used_pct")
        stats["elapsed_time_seconds"] = stats.get("elapsed_seconds")

        return stats
    
    def reset_policy_tracking(self):
        """Reset policy tracking (for new execution sessions)."""
        self.policy_enforcer.start_tracking()
        logger.info("Policy tracking reset")
    
    @staticmethod
    def _row_to_link(row: tuple) -> ExecutionLink:
        import json
        return ExecutionLink(
            link_id=row[0],
            caller_execution_id=row[1],
            callee_execution_id=row[2],
            caller_function_id=row[3],
            callee_function_id=row[4],
            invocation_order=row[5],
            context_snapshot=json.loads(row[6]) if row[6] else {},
            created_at=row[7],
        )


def create_execution_links_schema(cx: psycopg.Connection) -> None:
    """Create execution_links table and indexes if not exists.
    
    Args:
        cx: Active psycopg connection
    """
    with cx.cursor() as cur:
        # Create table
        cur.execute(
            """
            CREATE TABLE IF NOT EXISTS execution_links (
                link_id TEXT PRIMARY KEY,
                caller_execution_id TEXT NOT NULL,
                callee_execution_id TEXT NOT NULL,
                caller_function_id TEXT NOT NULL,
                callee_function_id TEXT NOT NULL,
                invocation_order INTEGER NOT NULL,
                context_snapshot JSONB,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                UNIQUE (caller_execution_id, invocation_order)
            )
            """
        )
        
        # Create indexes
        cur.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_execution_links_caller 
                ON execution_links(caller_execution_id, invocation_order)
            """
        )
        cur.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_execution_links_callee 
                ON execution_links(callee_execution_id, created_at DESC)
            """
        )
        cur.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_execution_links_function_pair 
                ON execution_links(caller_function_id, callee_function_id)
            """
        )
    
    cx.commit()
    logger.info("Created execution_links schema")
