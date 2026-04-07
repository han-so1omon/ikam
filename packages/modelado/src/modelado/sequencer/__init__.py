"""Sequencer module for planning and project phase management.

This module provides:
- IKAM artifact reference resolution and validation
- DAG validation for phase dependencies
- Estimation and confidence scoring
- Integration with instruction parser and MCP tools
"""

from modelado.sequencer.ikam_references import (
    resolve_ikam_references,
    validate_ikam_references,
    lookup_artifact_by_semantic_match,
)

from modelado.sequencer.models import (
    IKAMFragmentReference,
    PlanPhase,
    PhaseDependency,
    ValidationError,
    ValidationResult,
    EffortEstimate,
    CostEstimate,
    DurationEstimate,
    SequencerFragment,
    CommittedPhase,
    ProjectPhaseFragment,
)

from modelado.sequencer.validator import (
    validate_sequence,
    ValidationErrorCode,
    DAGValidationResult,
)

__all__ = [
    # IKAM reference functions
    "resolve_ikam_references",
    "validate_ikam_references",
    "lookup_artifact_by_semantic_match",
    # Validator functions
    "validate_sequence",
    "ValidationErrorCode",
    "DAGValidationResult",
    # Model classes
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
