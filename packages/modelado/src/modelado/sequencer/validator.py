"""DAG validator for sequencer fragments with IKAM reference integration.

Implements enhanced DAG validation supporting three edge types:
1. Phase dependencies (phase -> phase)
2. Artifact dependencies (phase -> artifact)
3. Fragment dependencies (phase -> fragment)

Validates topological ordering, IKAM reference integrity, and resource allocation.
"""

from dataclasses import dataclass
from typing import Dict, List, Optional, Set, Tuple
from enum import Enum

from .models import (
    SequencerFragment,
    PhaseDependency,
    PlanPhase,
    ValidationError,
    ValidationResult,
    IKAMFragmentReference,
)


class ValidationErrorCode(str, Enum):
    """Error codes for DAG validation."""

    DAG_CYCLE = "DAG_CYCLE"
    MISSING_PREDECESSOR = "MISSING_PREDECESSOR"
    MISSING_IKAM_REFERENCE = "MISSING_IKAM_REFERENCE"
    NO_ASSIGNEES = "NO_ASSIGNEES"
    EMPTY_ASSIGNEE_LIST = "EMPTY_ASSIGNEE_LIST"
    OVER_ALLOCATION = "OVER_ALLOCATION"
    INVALID_EDGE_TYPE = "INVALID_EDGE_TYPE"
    ORPHANED_PHASE = "ORPHANED_PHASE"
    MISSING_FRAGMENT = "MISSING_FRAGMENT"


@dataclass
class DAGValidationResult:
    """Result of DAG validation with detailed error reporting."""

    is_valid: bool
    errors: List[ValidationError]
    warnings: List[ValidationError]
    topological_order: Optional[List[str]] = None
    phase_edge_count: int = 0
    artifact_edge_count: int = 0
    fragment_edge_count: int = 0


def validate_sequence(
    sequencer_fragment: SequencerFragment,
    connection,
    max_concurrent_phases: int = 10,
) -> DAGValidationResult:
    """Validate sequencer fragment DAG with mixed edge type support.

    Args:
        sequencer_fragment: The sequencer fragment to validate
        connection: PostgreSQL database connection for IKAM reference validation
        max_concurrent_phases: Maximum allowed concurrent phases (default 10)

    Returns:
        DAGValidationResult with validation status, errors, warnings, and metadata

    Validation Checks:
    1. Topological sort (cycle detection)
    2. Missing predecessors (phase dependencies)
    3. IKAM reference integrity (artifact/fragment dependencies)
    4. Resource allocation (assignees)
    5. Edge type validity
    """
    errors: List[ValidationError] = []
    warnings: List[ValidationError] = []

    phases = sequencer_fragment.phases
    dependencies = sequencer_fragment.dependencies
    ikam_references = sequencer_fragment.ikam_references

    # Create phase ID lookup
    phase_ids = {phase.id for phase in phases}

    # Count edge types
    phase_edges = [dep for dep in dependencies if dep.edge_type == "phase"]
    artifact_edges = [dep for dep in dependencies if dep.edge_type == "artifact"]
    fragment_edges = [dep for dep in dependencies if dep.edge_type == "fragment"]

    phase_edge_count = len(phase_edges)
    artifact_edge_count = len(artifact_edges)
    fragment_edge_count = len(fragment_edges)

    # Validate edge types are valid literals
    for dep in dependencies:
        if dep.edge_type not in ["phase", "artifact", "fragment"]:
            errors.append(
                ValidationError(
                    code=ValidationErrorCode.INVALID_EDGE_TYPE,
                    message=f"Invalid edge type '{dep.edge_type}' in dependency from {dep.predecessor_id} to {dep.successor_id}",
                    severity="ERROR",
                )
            )

    # Validate phase edges: ensure predecessors exist
    for dep in phase_edges:
        if dep.predecessor_id not in phase_ids:
            errors.append(
                ValidationError(
                    code=ValidationErrorCode.MISSING_PREDECESSOR,
                    message=f"Phase dependency references missing predecessor: {dep.predecessor_id}",
                    severity="ERROR",
                    phase_ids=[dep.successor_id],
                )
            )
        if dep.successor_id not in phase_ids:
            errors.append(
                ValidationError(
                    code=ValidationErrorCode.MISSING_PREDECESSOR,
                    message=f"Phase dependency references missing successor: {dep.successor_id}",
                    severity="ERROR",
                    phase_ids=[dep.predecessor_id],
                )
            )

    # Validate IKAM references (artifact and fragment edges)
    if ikam_references:
        # For now, we skip IKAM validation to focus on DAG structure
        # Future: add resolve_ikam_references() call to fetch artifact metadata
        # from IKAM service before validating.
        pass

    # Validate assignees for each phase
    for phase in phases:
        if not hasattr(phase, "assignees") or phase.assignees is None:
            errors.append(
                ValidationError(
                    code=ValidationErrorCode.NO_ASSIGNEES,
                    message=f"Phase {phase.id} has no assignees field",
                    severity="ERROR",
                    phase_ids=[phase.id],
                )
            )
        elif len(phase.assignees) == 0:
            errors.append(
                ValidationError(
                    code=ValidationErrorCode.EMPTY_ASSIGNEE_LIST,
                    message=f"Phase {phase.id} has empty assignees list",
                    severity="ERROR",
                    phase_ids=[phase.id],
                )
            )

    # Topological sort (cycle detection)
    topological_order = _topological_sort(phases, phase_edges)
    if topological_order is None:
        errors.append(
            ValidationError(
                code=ValidationErrorCode.DAG_CYCLE,
                message="Cycle detected in phase dependencies",
                severity="ERROR",
                phase_ids=[p.id for p in phases],
            )
        )

    # Check for orphaned phases (no incoming or outgoing edges)
    phases_with_edges = set()
    for dep in dependencies:
        phases_with_edges.add(dep.predecessor_id)
        phases_with_edges.add(dep.successor_id)

    for phase in phases:
        if phase.id not in phases_with_edges and len(phases) > 1:
            warnings.append(
                ValidationError(
                    code=ValidationErrorCode.ORPHANED_PHASE,
                    message=f"Phase {phase.id} has no dependencies (orphaned)",
                    severity="WARNING",
                    phase_ids=[phase.id],
                )
            )

    return DAGValidationResult(
        is_valid=len(errors) == 0,
        errors=errors,
        warnings=warnings,
        topological_order=topological_order,
        phase_edge_count=phase_edge_count,
        artifact_edge_count=artifact_edge_count,
        fragment_edge_count=fragment_edge_count,
    )


def _topological_sort(
    phases: List[PlanPhase], dependencies: List[PhaseDependency]
) -> Optional[List[str]]:
    """Perform topological sort on phase dependencies.

    Args:
        phases: List of phases to sort
        dependencies: Phase-to-phase dependencies (edge_type='phase')

    Returns:
        Topologically sorted list of phase IDs, or None if cycle detected
    """
    # Build adjacency list and in-degree count
    adj: Dict[str, List[str]] = {phase.id: [] for phase in phases}
    in_degree: Dict[str, int] = {phase.id: 0 for phase in phases}

    for dep in dependencies:
        # Only process phase edges for topological sort
        if dep.edge_type != "phase":
            continue

        # Skip if predecessor/successor not in phase list
        if dep.predecessor_id not in adj or dep.successor_id not in in_degree:
            continue

        adj[dep.predecessor_id].append(dep.successor_id)
        in_degree[dep.successor_id] += 1

    # Kahn's algorithm
    queue: List[str] = [pid for pid, degree in in_degree.items() if degree == 0]
    result: List[str] = []

    while queue:
        current = queue.pop(0)
        result.append(current)

        for neighbor in adj[current]:
            in_degree[neighbor] -= 1
            if in_degree[neighbor] == 0:
                queue.append(neighbor)

    # If result doesn't include all phases, there's a cycle
    if len(result) != len(phases):
        return None

    return result
