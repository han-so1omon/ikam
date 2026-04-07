"""Sequencer and simulator data models.

This module defines the core data structures for project planning:
- SequencerFragment: Planning artifact with phases, dependencies, IKAM references, estimates
- ProjectPhaseFragment: Committed project phases (output of sequencer execution)
- IKAMFragmentReference: External IKAM artifact/fragment references
- Supporting types: PlanPhase, PhaseDependency, ValidationResult, Estimates

All models use Pydantic BaseModel for validation and JSON schema support.
"""

from __future__ import annotations

from datetime import datetime
from typing import Any, Dict, List, Literal, Optional
from pydantic import BaseModel, Field
from uuid import UUID


# ============================================================================
# IKAM Reference Support
# ============================================================================

class IKAMFragmentReference(BaseModel):
    """
    Reference to an external IKAM artifact or fragment.
    
    Enables phases to reference economic models, variables, equations, data,
    narratives, and other IKAM fragments for explicit context binding.
    
    Examples:
    - uses_variable: Phase references a cost driver variable from economic model
    - depends_on_formula: Phase depends on break-even calculation from sheet
    - input_from_data: Phase consumes forecast data from imported CSV
    - output_to_narrative: Phase produces narrative section for investor deck
    - extends_model: Phase builds upon existing economic model artifact
    """
    
    artifact_id: str = Field(
        ...,
        description="UUID of referenced IKAM artifact (economic model, sheet, narrative, etc.)"
    )
    
    fragment_id: Optional[str] = Field(
        default=None,
        description="Optional fragment ID if referencing specific decomposed piece"
    )
    
    reference_type: Literal[
        "uses_variable",
        "depends_on_formula",
        "input_from_data",
        "output_to_narrative",
        "extends_model"
    ] = Field(
        ...,
        description="Type of reference relationship between phase and IKAM artifact"
    )
    
    scope: List[str] = Field(
        default_factory=list,
        description="Phase IDs that use this reference"
    )
    
    metadata: Dict[str, Any] = Field(
        default_factory=dict,
        description="Binding info (e.g., which phase var maps to external var, confidence, artifact_kind, artifact_title)"
    )


# ============================================================================
# Phase Definitions
# ============================================================================

class PlanPhase(BaseModel):
    """
    Individual phase in a project plan.
    
    Phases can have IKAM inputs (artifacts/fragments consumed) and outputs
    (artifacts/fragments produced) for explicit data flow tracking.
    """
    
    id: str = Field(..., description="Unique phase identifier")
    title: str = Field(..., description="Human-readable phase name")
    description: str = Field(default="", description="Phase description")
    
    # Effort estimation
    estimated_effort: float = Field(
        ...,
        ge=0,
        description="Estimated effort in person-days"
    )
    
    # Assignments
    assignees: List[str] = Field(
        default_factory=list,
        description="List of assignee IDs (user IDs or role names)"
    )
    
    # Scheduling (optional, can be derived from dependencies)
    phase_start: Optional[datetime] = Field(
        default=None,
        description="Planned start date (optional, can be computed)"
    )
    phase_end: Optional[datetime] = Field(
        default=None,
        description="Planned end date (optional, can be computed)"
    )
    
    # IKAM integration
    ikam_inputs: List[str] = Field(
        default_factory=list,
        description="IKAM artifact IDs consumed by this phase"
    )
    
    ikam_outputs: List[str] = Field(
        default_factory=list,
        description="IKAM artifact IDs produced by this phase"
    )
    
    # Risk/confidence
    risk_score: float = Field(
        default=0.5,
        ge=0.0,
        le=1.0,
        description="Risk score for this phase (0=low, 1=high)"
    )


class PhaseDependency(BaseModel):
    """
    Dependency edge between two phases or between phase and IKAM artifact.
    
    Supports 3 edge types:
    - phase: Traditional phase → phase dependency (phase A must complete before phase B)
    - artifact: Phase depends on entire IKAM artifact (e.g., needs cost model)
    - fragment: Phase depends on specific IKAM fragment (e.g., needs specific formula)
    """
    
    predecessor_id: str = Field(
        ...,
        description="ID of predecessor (phase ID or artifact ID depending on edge_type)"
    )
    
    successor_id: str = Field(
        ...,
        description="ID of successor phase"
    )
    
    dependency_type: str = Field(
        default="finish_to_start",
        description="Type of dependency (finish_to_start, start_to_start, etc.)"
    )
    
    edge_type: Literal["phase", "artifact", "fragment"] = Field(
        default="phase",
        description="Type of dependency edge (phase-to-phase, artifact, or fragment)"
    )


# ============================================================================
# Validation
# ============================================================================

class ValidationError(BaseModel):
    """Single validation error or warning."""
    
    code: str = Field(..., description="Error code (DAG_CYCLE, MISSING_PREDECESSOR, etc.)")
    message: str = Field(..., description="Human-readable error message")
    severity: Literal["ERROR", "WARNING"] = Field(..., description="Error severity")
    phase_ids: List[str] = Field(default_factory=list, description="Affected phase IDs")


class ValidationResult(BaseModel):
    """Result of validating a sequencer fragment."""
    
    is_valid: bool = Field(..., description="True if validation passed with no errors")
    errors: List[ValidationError] = Field(default_factory=list, description="List of validation errors")
    warnings: List[ValidationError] = Field(default_factory=list, description="List of validation warnings")


# ============================================================================
# Estimation
# ============================================================================

class EffortEstimate(BaseModel):
    """Effort estimation with simple/medium/complex variants."""
    
    simple_estimate: float = Field(..., ge=0, description="Simple estimation (person-days)")
    medium_estimate: float = Field(..., ge=0, description="Medium estimation with role breakdown")
    complex_estimate: float = Field(..., ge=0, description="Complex estimation with risk adjustment")
    unit: str = Field(default="person-days", description="Unit of effort")


class CostEstimate(BaseModel):
    """Cost estimation with base/role-based/risk-adjusted variants."""
    
    base_cost: float = Field(..., ge=0, description="Base cost (effort * average rate)")
    role_based_cost: float = Field(..., ge=0, description="Role-based cost (effort * role-specific rates)")
    risk_adjusted_cost: float = Field(..., ge=0, description="Risk-adjusted cost (role-based + contingency)")
    currency: str = Field(default="USD", description="Currency code")


class DurationEstimate(BaseModel):
    """Duration estimation with optimistic/nominal/pessimistic variants."""
    
    optimistic: float = Field(..., ge=0, description="Optimistic duration (days)")
    nominal: float = Field(..., ge=0, description="Nominal duration (days)")
    pessimistic: float = Field(..., ge=0, description="Pessimistic duration (days)")
    critical_path_days: float = Field(..., ge=0, description="Critical path duration (days)")


# ============================================================================
# Main Fragment Types
# ============================================================================

class SequencerFragment(BaseModel):
    """
    Planning artifact with effort/cost/risk estimates.
    
    Phases can reference other IKAM fragments (variables, artifacts, equations, data)
    to ensure planning context is explicit and traceable.
    
    This is the primary output of the MCP Sequencer service (create_sequence tool).
    """
    
    # Structure
    phases: List[PlanPhase] = Field(
        ...,
        description="Ordered list of project phases"
    )
    
    dependencies: List[PhaseDependency] = Field(
        default_factory=list,
        description="Phase → phase and artifact → phase edges"
    )
    
    assignments: Dict[str, List[str]] = Field(
        default_factory=dict,
        description="phase_id → assignee_ids mapping"
    )
    
    # IKAM Fragment References
    ikam_references: List[IKAMFragmentReference] = Field(
        default_factory=list,
        description="External IKAM artifacts/fragments referenced by phases"
    )
    
    # Validation results
    validation: ValidationResult = Field(
        ...,
        description="DAG check, assignee check, allocation check, fragment availability"
    )
    
    # Estimates (multiple modes)
    effort_estimate: EffortEstimate = Field(
        ...,
        description="Effort estimates in simple/medium/complex variants"
    )
    
    cost_estimate: CostEstimate = Field(
        ...,
        description="Cost estimates in base/role-based/risk-adjusted variants"
    )
    
    duration_estimate: DurationEstimate = Field(
        ...,
        description="Duration estimates with critical path"
    )
    
    # Risk/confidence aggregation
    risk_score: float = Field(
        default=0.5,
        ge=0.0,
        le=1.0,
        description="Aggregated risk score across phases (0=low, 1=high)"
    )
    
    confidence_score: float = Field(
        default=0.5,
        ge=0.0,
        le=1.0,
        description="Estimation confidence (0=low, 1=high), adjusted by IKAM reference availability"
    )
    
    # Provenance
    derived_from_instruction_id: Optional[str] = Field(
        default=None,
        description="Link back to instruction if from planning request"
    )
    
    # Metadata
    requested_by: str = Field(..., description="User ID or agent ID that requested this plan")
    request_mode: Literal["simple", "medium", "complex"] = Field(
        ...,
        description="Estimation mode used (simple | medium | complex)"
    )
    created_at: datetime = Field(
        default_factory=datetime.utcnow,
        description="Timestamp when fragment was created"
    )


class CommittedPhase(BaseModel):
    """Phase that has been committed to execution (subset of PlanPhase)."""
    
    id: str
    title: str
    description: str
    estimated_effort: float
    assignees: List[str]
    phase_start: Optional[datetime] = None
    phase_end: Optional[datetime] = None
    ikam_inputs: List[str] = Field(default_factory=list)
    ikam_outputs: List[str] = Field(default_factory=list)
    status: Literal["planned", "in_progress", "completed"] = Field(default="planned")


class ProjectPhaseFragment(BaseModel):
    """
    Committed project phases (output of sequencer execution).
    
    This is created when user confirms a SequencerFragment plan and commits
    it to execution. Includes full provenance chain back to the sequencer.
    """
    
    # Phase data (from sequencer)
    phases: List[CommittedPhase] = Field(
        ...,
        description="Committed phases from sequencer plan"
    )
    
    dependencies: List[PhaseDependency] = Field(
        default_factory=list,
        description="Phase dependencies from sequencer plan"
    )
    
    assignments: Dict[str, List[str]] = Field(
        default_factory=dict,
        description="phase_id → assignee_ids mapping"
    )
    
    # Provenance
    derived_from: str = Field(
        ...,
        description="SequencerFragment ID this was committed from"
    )
    
    derivation_type: str = Field(
        default="sequencer_fragment_committed",
        description="Derivation type for provenance tracking"
    )
    
    sequencer_request_id: str = Field(
        ...,
        description="Original sequencer request ID for tracing back to instruction"
    )
    
    # Execution state
    status: Literal["planned", "in_progress", "completed"] = Field(
        default="planned",
        description="Overall project status"
    )
    
    phases_completed: List[str] = Field(
        default_factory=list,
        description="Phase IDs that are completed"
    )
    
    # Metadata
    committed_by: str = Field(..., description="User ID who committed the plan")
    committed_at: datetime = Field(
        default_factory=datetime.utcnow,
        description="Timestamp when plan was committed"
    )


# ============================================================================
# Export all public types
# ============================================================================

__all__ = [
    "IKAMFragmentReference",
    "PlanPhase",
    "PhaseDependency",
    "ValidationError",
    "ValidationResult",
    "EffortEstimate",
    "CostEstimate",
    "DurationEstimate",
    "SequencerFragment",
    "CommittedPhase",
    "ProjectPhaseFragment",
]
