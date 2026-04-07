"""Effort, duration, and cost estimation for sequencer fragments.

Implements three estimation strategies:
1. Duration: Simple (sum) and critical path (longest DAG path)
2. Effort: Simple (sum) and Fibonacci (complexity-based story points)
3. Cost: Simple (flat rate) and role-based (per-role rates with risk adjustment)
"""

from typing import Dict, List, Optional, Tuple
from .models import (
    SequencerFragment,
    PlanPhase,
    PhaseDependency,
    EffortEstimate,
    CostEstimate,
    DurationEstimate,
)


# Fibonacci sequence for effort estimation (story points)
FIBONACCI_SEQUENCE = [1, 2, 3, 5, 8, 13, 21, 34, 55, 89]

# Role-based rates ($/person-day)
ROLE_RATES = {
    "engineer": 150.0,
    "designer": 100.0,
    "pm": 120.0,
    "developer": 150.0,
    "qa": 100.0,
    "contractor": 200.0,
}


def estimate_duration(
    fragment: SequencerFragment,
    mode: str = "simple",
) -> DurationEstimate:
    """Estimate project duration.

    Args:
        fragment: SequencerFragment to estimate
        mode: "simple" (sum) or "critical_path" (longest DAG path)

    Returns:
        DurationEstimate with optimistic, nominal, pessimistic, and critical path values
    """
    if mode == "simple":
        # Sum all phase efforts (assumes sequential)
        total_days = sum(p.estimated_effort for p in fragment.phases)
        return DurationEstimate(
            optimistic=total_days * 0.8,
            nominal=total_days,
            pessimistic=total_days * 1.2,
            critical_path_days=total_days,
        )

    elif mode == "critical_path":
        # Find longest path in DAG using topological sort
        cp_length = _critical_path_analysis(
            fragment.phases, fragment.dependencies
        )
        return DurationEstimate(
            optimistic=cp_length * 0.8,
            nominal=cp_length,
            pessimistic=cp_length * 1.2,
            critical_path_days=cp_length,
        )

    else:
        raise ValueError(f"Unknown duration mode: {mode}")


def estimate_effort(
    fragment: SequencerFragment,
    mode: str = "simple",
) -> EffortEstimate:
    """Estimate effort in person-days or story points.

    Args:
        fragment: SequencerFragment to estimate
        mode: "simple" (sum) or "fibonacci" (complexity-based story points)

    Returns:
        EffortEstimate with simple, medium, and complex variants
    """
    if mode == "simple":
        # Sum all phase efforts
        total = sum(p.estimated_effort for p in fragment.phases)
        return EffortEstimate(
            simple_estimate=total,
            medium_estimate=total * 1.25,
            complex_estimate=total * 1.5,
        )

    elif mode == "fibonacci":
        # Assign Fibonacci values based on phase complexity
        estimates = []
        for phase in fragment.phases:
            complexity = _estimate_phase_complexity(phase)
            # Clamp complexity to valid Fibonacci index
            complexity = min(complexity, len(FIBONACCI_SEQUENCE) - 1)
            fib_value = FIBONACCI_SEQUENCE[complexity]
            estimates.append(fib_value)

        total = sum(estimates)
        return EffortEstimate(
            simple_estimate=float(total),
            medium_estimate=total * 1.5,
            complex_estimate=total * 2.0,
        )

    else:
        raise ValueError(f"Unknown effort mode: {mode}")


def estimate_cost(
    fragment: SequencerFragment,
    mode: str = "simple",
) -> CostEstimate:
    """Estimate project cost.

    Args:
        fragment: SequencerFragment to estimate
        mode: "simple" (flat rate) or "role_based" (per-role rates with risk adjustment)

    Returns:
        CostEstimate with base, role-based, and risk-adjusted costs
    """
    if mode == "simple":
        # Simple: sum effort * flat rate
        total_days = sum(p.estimated_effort for p in fragment.phases)
        base_cost = total_days * 150.0  # $150/day average
        risk_premium = 1.0 + (fragment.risk_score * 0.2)  # 0-20% premium
        return CostEstimate(
            base_cost=base_cost,
            role_based_cost=base_cost * 1.0,  # Same as base for simple mode
            risk_adjusted_cost=base_cost * risk_premium,
        )

    elif mode == "role_based":
        # Role-based: per-role rates based on assignees
        cost_by_role: Dict[str, float] = {}
        total_base_cost = 0.0

        for phase in fragment.phases:
            assignees = fragment.assignments.get(phase.id, [])
            effort_per_assignee = (
                phase.estimated_effort / len(assignees)
                if assignees
                else phase.estimated_effort
            )

            for assignee in assignees:
                role = _infer_role(assignee)
                rate = ROLE_RATES.get(role, 150.0)
                cost = effort_per_assignee * rate
                cost_by_role[role] = cost_by_role.get(role, 0.0) + cost
                total_base_cost += cost

        # Apply risk premium
        risk_premium = 1.0 + (fragment.risk_score * 0.2)  # 0-20% premium
        return CostEstimate(
            base_cost=total_base_cost,
            role_based_cost=total_base_cost,
            risk_adjusted_cost=total_base_cost * risk_premium,
        )

    else:
        raise ValueError(f"Unknown cost mode: {mode}")


def aggregate_risk_confidence(
    fragment: SequencerFragment,
) -> Tuple[float, float]:
    """Aggregate risk and confidence across all phases.

    Args:
        fragment: SequencerFragment to aggregate

    Returns:
        Tuple of (risk_score, confidence_score) both in [0, 1]
    """
    if not fragment.phases:
        return 0.5, 0.5

    # Risk: max risk in any phase (pessimistic aggregation)
    risk_score = max(
        (getattr(p, "risk_score", 0.5) for p in fragment.phases),
        default=0.5,
    )

    # Confidence: average of phase confidences
    confidences = [
        getattr(p, "confidence_score", 0.7) for p in fragment.phases
    ]
    confidence_score = sum(confidences) / len(confidences) if confidences else 0.7

    return risk_score, confidence_score


def _critical_path_analysis(
    phases: List[PlanPhase],
    dependencies: List[PhaseDependency],
) -> float:
    """Calculate critical path (longest path) in phase DAG.

    Args:
        phases: List of phases
        dependencies: List of phase dependencies

    Returns:
        Length of critical path (sum of efforts on longest path)
    """
    if not phases:
        return 0.0

    # Build phase lookup and adjacency
    phase_map = {p.id: p for p in phases}
    phase_ids = {p.id for p in phases}

    # Only process phase-to-phase dependencies
    edges = [
        d
        for d in dependencies
        if d.edge_type == "phase"
        and d.predecessor_id in phase_ids
        and d.successor_id in phase_ids
    ]

    # Topological sort with path tracking
    in_degree = {pid: 0 for pid in phase_ids}
    adjacency = {pid: [] for pid in phase_ids}

    for edge in edges:
        adjacency[edge.predecessor_id].append(edge.successor_id)
        in_degree[edge.successor_id] += 1

    # Kahn's algorithm with path tracking
    queue = [pid for pid, degree in in_degree.items() if degree == 0]
    path_lengths = {pid: phase_map[pid].estimated_effort for pid in phase_ids}

    while queue:
        current = queue.pop(0)
        for neighbor in adjacency[current]:
            in_degree[neighbor] -= 1
            # Update longest path to neighbor
            path_lengths[neighbor] = max(
                path_lengths[neighbor],
                path_lengths[current] + phase_map[neighbor].estimated_effort,
            )
            if in_degree[neighbor] == 0:
                queue.append(neighbor)

    # Return longest path length
    return max(path_lengths.values()) if path_lengths else 0.0


def _estimate_phase_complexity(phase: PlanPhase) -> int:
    """Estimate phase complexity (0-9) based on effort and assignees.

    Args:
        phase: Phase to estimate

    Returns:
        Complexity index (0-9) for Fibonacci sequence selection
    """
    # Simple heuristic: effort + team size
    effort_factor = min(int(phase.estimated_effort / 5), 5)
    team_size = len(getattr(phase, "assignees", []))
    team_factor = min(team_size // 2, 2)

    complexity = effort_factor + team_factor
    return min(complexity, 9)  # Clamp to valid Fibonacci index


def _infer_role(assignee: str) -> str:
    """Infer role from assignee ID/name.

    Args:
        assignee: Assignee ID or name

    Returns:
        Inferred role (engineer, designer, pm, etc.)
    """
    assignee_lower = assignee.lower()

    # Simple pattern matching
    if any(x in assignee_lower for x in ["eng", "dev", "programmer", "developer"]):
        return "engineer"
    elif any(x in assignee_lower for x in ["design", "ui", "ux"]):
        return "designer"
    elif any(x in assignee_lower for x in ["pm", "product", "manager"]):
        return "pm"
    elif any(x in assignee_lower for x in ["qa", "test", "qc"]):
        return "qa"
    elif "contractor" in assignee_lower:
        return "contractor"
    else:
        return "engineer"  # Default
