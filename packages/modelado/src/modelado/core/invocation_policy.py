"""Invocation policy limits for generative operations.

This module implements depth limits, fan-out caps, and budget enforcement
for execution call graphs to prevent runaway generation and ensure bounded costs.
"""

from dataclasses import dataclass
from typing import Any, Dict, List, Optional, Set
from datetime import datetime
import logging

logger = logging.getLogger(__name__)


@dataclass
class InvocationPolicy:
    """Policy limits for generative function invocations.
    
    Enforces:
    - Maximum call depth (prevents infinite recursion)
    - Maximum fan-out per caller (prevents combinatorial explosion)
    - Cost budget limits (prevents runaway spending)
    - Execution time limits (prevents long-running chains)
    
    Mathematical guarantee: Total invocations ≤ Σ(max_fan_out^d) for d ∈ [0, max_depth]
    Example: max_depth=3, max_fan_out=5 → max invocations ≤ 1 + 5 + 25 + 125 = 156
    """
    
    max_depth: int = 3
    """Maximum call depth (0 = root only, 3 = root + 3 levels of callees)."""
    
    max_fan_out: int = 10
    """Maximum number of callees per caller (prevents wide trees)."""
    
    max_total_cost: float = 100.0
    """Maximum total cost in USD for all model calls in execution tree."""
    
    max_execution_time_seconds: float = 300.0
    """Maximum wall-clock time for entire execution tree (5 minutes default)."""
    
    allow_cycles: bool = False
    """Whether to allow cycles in call graph (default: False for safety)."""
    
    max_total_executions: Optional[int] = None
    """Optional hard cap on total number of executions (overrides depth/fan-out)."""
    
    def __post_init__(self):
        """Validate policy parameters."""
        if self.max_depth < 0:
            raise ValueError(f"max_depth must be ≥ 0, got {self.max_depth}")
        if self.max_fan_out < 1:
            raise ValueError(f"max_fan_out must be ≥ 1, got {self.max_fan_out}")
        if self.max_total_cost <= 0:
            raise ValueError(f"max_total_cost must be > 0, got {self.max_total_cost}")
        if self.max_execution_time_seconds <= 0:
            raise ValueError(f"max_execution_time_seconds must be > 0, got {self.max_execution_time_seconds}")
    
    def estimate_max_executions(self) -> int:
        """Estimate maximum possible executions given depth and fan-out limits.
        
        Uses geometric series: Σ(r^d) = (r^(n+1) - 1) / (r - 1) for r ≠ 1
        For r = max_fan_out, d ∈ [0, max_depth]:
        max_executions = (max_fan_out^(max_depth+1) - 1) / (max_fan_out - 1)
        
        Returns:
            Upper bound on executions (assumes full tree saturation)
        """
        if self.max_fan_out == 1:
            # Linear chain: 1 + 1 + 1 + ... = max_depth + 1
            return self.max_depth + 1
        
        # Geometric series formula
        r = self.max_fan_out
        n = self.max_depth
        max_from_tree = int((r ** (n + 1) - 1) / (r - 1))
        
        # Apply hard cap if specified
        if self.max_total_executions is not None:
            return min(max_from_tree, self.max_total_executions)
        
        return max_from_tree


class PolicyViolation(Exception):
    """Raised when an invocation graph violates policy constraints."""

    def __init__(
        self,
        violation_type: str,
        message: str,
        *,
        execution_id: Optional[str] = None,
        caller_execution_id: Optional[str] = None,
        current_value: Optional[float] = None,
        limit_value: Optional[float] = None,
        timestamp: Optional[datetime] = None,
    ):
        super().__init__(message)
        self.violation_type = violation_type
        self.message = message
        self.execution_id = execution_id
        self.caller_execution_id = caller_execution_id
        self.current_value = current_value
        self.limit_value = limit_value
        self.timestamp = timestamp or datetime.utcnow()


class PolicyEnforcer:
    """Enforces invocation policies during execution graph construction.
    
    Usage:
        policy = InvocationPolicy(max_depth=3, max_fan_out=5, max_total_cost=50.0)
        enforcer = PolicyEnforcer(policy)
        
        # Before adding execution link
        violation = enforcer.check_before_add_link(
            caller_execution_id="exec_1",
            caller_depth=2,
            current_fan_out=3,
            new_callee_cost=1.5
        )
        
        if violation:
            logger.error(f"Policy violation: {violation.message}")
            raise RuntimeError(violation.message)
    """
    
    def __init__(
        self,
        policy: Optional[InvocationPolicy] = None,
        *,
        max_depth: Optional[int] = None,
        max_fanout: Optional[int] = None,
        max_total_cost: Optional[float] = None,
        max_execution_time_seconds: Optional[float] = None,
        allow_cycles: Optional[bool] = None,
    ):
        # Allow the legacy test API: PolicyEnforcer(max_depth=..., max_fanout=...)
        # while preserving the InvocationPolicy-based API.
        self.policy = policy or InvocationPolicy()
        if max_depth is not None:
            self.policy.max_depth = max_depth
        if max_fanout is not None:
            self.policy.max_fan_out = max_fanout
        if max_total_cost is not None:
            self.policy.max_total_cost = max_total_cost
        if max_execution_time_seconds is not None:
            self.policy.max_execution_time_seconds = max_execution_time_seconds
        if allow_cycles is not None:
            self.policy.allow_cycles = allow_cycles

        self.total_cost = 0.0
        self.total_executions = 0
        self.start_time: Optional[datetime] = None
        self.violations: List[PolicyViolation] = []

    def validate_graph(self, graph: Any) -> None:
        """Validate an in-memory execution graph.

        Tests expect `graph.graph` to be a mapping of caller -> {callee -> order}.
        Raises PolicyViolation on first failure.
        """
        g = getattr(graph, "graph", None)
        if not isinstance(g, dict):
            raise PolicyViolation("invalid_graph", "Graph is missing required 'graph' mapping")

        # Fan-out check
        for caller_id, children in g.items():
            if not isinstance(children, dict):
                raise PolicyViolation("invalid_graph", f"Graph children for {caller_id} must be a dict")
            if len(children) > self.policy.max_fan_out:
                raise PolicyViolation(
                    "fan-out",
                    f"fan-out {len(children)} exceeds limit {self.policy.max_fan_out}",
                    caller_execution_id=str(caller_id),
                    current_value=float(len(children)),
                    limit_value=float(self.policy.max_fan_out),
                )

        # Depth check
        depths = graph.compute_depths() if hasattr(graph, "compute_depths") else {}
        if depths:
            max_depth_val = max(depths.values())
            if max_depth_val > self.policy.max_depth:
                raise PolicyViolation(
                    "depth",
                    f"depth {max_depth_val} exceeds limit {self.policy.max_depth}",
                    current_value=float(max_depth_val),
                    limit_value=float(self.policy.max_depth),
                )
    
    def start_tracking(self):
        """Start tracking execution time."""
        self.start_time = datetime.utcnow()
        self.total_cost = 0.0
        self.total_executions = 0
        self.violations.clear()
    
    def check_before_add_link(
        self,
        caller_execution_id: str,
        callee_depth: int,
        current_fan_out: int,
        new_callee_cost: float = 0.0,
    ) -> Optional[PolicyViolation]:
        """Check if adding a new execution link would violate policy.
        
        Args:
            caller_execution_id: ID of caller execution
            callee_depth: Depth of new callee (caller_depth + 1)
            current_fan_out: Current number of callees for this caller
            new_callee_cost: Estimated cost of new callee execution
            
        Returns:
            PolicyViolation if limit would be exceeded, None otherwise
        """
        # Check depth limit
        if callee_depth > self.policy.max_depth:
            violation = PolicyViolation(
                "max_depth",
                f"Depth {callee_depth} exceeds limit {self.policy.max_depth}",
                caller_execution_id=caller_execution_id,
                current_value=float(callee_depth),
                limit_value=float(self.policy.max_depth),
            )
            self.violations.append(violation)
            return violation
        
        # Check fan-out limit (before adding new child)
        if current_fan_out >= self.policy.max_fan_out:
            violation = PolicyViolation(
                "max_fan_out",
                f"Fan-out {current_fan_out + 1} exceeds limit {self.policy.max_fan_out}",
                caller_execution_id=caller_execution_id,
                current_value=float(current_fan_out + 1),
                limit_value=float(self.policy.max_fan_out),
            )
            self.violations.append(violation)
            return violation
        
        # Check cost budget
        projected_cost = self.total_cost + new_callee_cost
        if projected_cost > self.policy.max_total_cost:
            violation = PolicyViolation(
                "max_cost",
                f"Projected cost ${projected_cost:.2f} exceeds budget ${self.policy.max_total_cost:.2f}",
                current_value=projected_cost,
                limit_value=self.policy.max_total_cost,
            )
            self.violations.append(violation)
            return violation
        
        # Check execution count limit
        if self.policy.max_total_executions is not None:
            if self.total_executions >= self.policy.max_total_executions:
                violation = PolicyViolation(
                    "max_executions",
                    f"Execution count {self.total_executions + 1} exceeds limit {self.policy.max_total_executions}",
                    current_value=float(self.total_executions + 1),
                    limit_value=float(self.policy.max_total_executions),
                )
                self.violations.append(violation)
                return violation
        
        # Check execution time
        if self.start_time is not None:
            elapsed = (datetime.utcnow() - self.start_time).total_seconds()
            if elapsed > self.policy.max_execution_time_seconds:
                violation = PolicyViolation(
                    "max_time",
                    f"Execution time {elapsed:.1f}s exceeds limit {self.policy.max_execution_time_seconds:.1f}s",
                    current_value=elapsed,
                    limit_value=self.policy.max_execution_time_seconds,
                )
                self.violations.append(violation)
                return violation
        
        return None
    
    def record_execution(self, cost: float = 0.0):
        """Record execution of a function (updates cost and count).
        
        Args:
            cost: Cost of this execution in USD
        """
        self.total_cost += cost
        self.total_executions += 1
    
    def check_cycle(
        self,
        caller_execution_id: str,
        callee_execution_id: str,
        execution_graph: Dict[str, List[str]],
    ) -> Optional[PolicyViolation]:
        """Check if adding edge would create a cycle.
        
        Args:
            caller_execution_id: ID of caller
            callee_execution_id: ID of callee
            execution_graph: Current graph as adjacency list
            
        Returns:
            PolicyViolation if cycle detected and not allowed, None otherwise
        """
        if self.policy.allow_cycles:
            return None
        
        # DFS to check if callee can reach caller
        visited = set()
        stack = [callee_execution_id]
        
        while stack:
            current = stack.pop()
            if current in visited:
                continue
            
            visited.add(current)
            
            # If callee can reach caller, adding edge creates cycle
            if current == caller_execution_id:
                violation = PolicyViolation(
                    violation_type="cycle_detected",
                    message=f"Adding edge {caller_execution_id}→{callee_execution_id} creates cycle",
                    caller_execution_id=caller_execution_id,
                    execution_id=callee_execution_id,
                )
                self.violations.append(violation)
                return violation
            
            # Add children to stack
            if current in execution_graph:
                stack.extend(execution_graph[current])
        
        return None
    
    def get_statistics(self) -> Dict[str, Any]:
        """Get current execution statistics.
        
        Returns:
            Dictionary with current metrics and policy limits
        """
        elapsed = None
        if self.start_time is not None:
            elapsed = (datetime.utcnow() - self.start_time).total_seconds()
        
        return {
            "total_cost": self.total_cost,
            "total_executions": self.total_executions,
            "elapsed_seconds": elapsed,
            "violations": len(self.violations),
            "policy": {
                "max_depth": self.policy.max_depth,
                "max_fan_out": self.policy.max_fan_out,
                "max_total_cost": self.policy.max_total_cost,
                "max_execution_time_seconds": self.policy.max_execution_time_seconds,
                "max_total_executions": self.policy.max_total_executions,
            },
            "utilization": {
                "cost_used_pct": (self.total_cost / self.policy.max_total_cost) * 100,
                "time_used_pct": (elapsed / self.policy.max_execution_time_seconds * 100) if elapsed else None,
                "executions_used_pct": (
                    (self.total_executions / self.policy.max_total_executions * 100)
                    if self.policy.max_total_executions else None
                ),
            },
        }


# Default policies for common use cases
DEFAULT_POLICY = InvocationPolicy(
    max_depth=3,
    max_fan_out=10,
    max_total_cost=100.0,
    max_execution_time_seconds=300.0,
)

CONSERVATIVE_POLICY = InvocationPolicy(
    max_depth=2,
    max_fan_out=5,
    max_total_cost=10.0,
    max_execution_time_seconds=60.0,
)

GENEROUS_POLICY = InvocationPolicy(
    max_depth=5,
    max_fan_out=20,
    max_total_cost=500.0,
    max_execution_time_seconds=600.0,
)

TESTING_POLICY = InvocationPolicy(
    max_depth=2,
    max_fan_out=3,
    max_total_cost=1.0,
    max_execution_time_seconds=10.0,
    allow_cycles=True,
)
