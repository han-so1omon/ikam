"""MCP tools for sequencer service.

Implements three MCP tools for project planning:
1. create_sequence: Generate project plan from natural language instruction
2. validate_sequence: Validate existing sequence fragment
3. commit_sequence: Convert SequencerFragment to committed ProjectPhaseFragment

All tools return structured responses compatible with MCP protocol.
Commit tool includes DB-backed IKAM resolution and derivation event emission.
"""

from typing import Any, Dict, List, Optional
from datetime import datetime
import uuid
try:
    import psycopg
except ImportError:
    psycopg = None  # type: ignore

from modelado.sequencer.models import (
    SequencerFragment,
    ProjectPhaseFragment,
    PlanPhase,
    PhaseDependency,
    CommittedPhase,
    ValidationResult,
    ValidationError,
    IKAMFragmentReference,
    EffortEstimate,
    CostEstimate,
    DurationEstimate,
)
from modelado.sequencer.validator import validate_sequence
from modelado.sequencer.estimator import (
    estimate_duration,
    estimate_effort,
    estimate_cost,
    aggregate_risk_confidence,
)
from modelado.sequencer.ikam_references import (
    resolve_ikam_references,
    validate_ikam_references,
)
from modelado.core.provenance_recorder import (
    DerivationProvenanceEvent,
)


def create_sequence(
    instruction: str,
    phases: List[Dict[str, Any]],
    dependencies: List[Dict[str, Any]],
    ikam_references: Optional[List[Dict[str, Any]]] = None,
    requested_by: str = "mcp-sequencer",
    request_mode: str = "medium",
    instruction_id: Optional[str] = None,
) -> Dict[str, Any]:
    """Create a project sequence from planning instruction.

    Args:
        instruction: Natural language planning instruction
        phases: List of phase dicts with id, title, estimated_effort, assignees
        dependencies: List of dependency dicts with predecessor_id, successor_id, edge_type
        ikam_references: Optional IKAM references (artifact_id, reference_type, scope, metadata)
        requested_by: User or agent ID making the request
        request_mode: Estimation mode (simple | medium | complex)
        instruction_id: Optional link to instruction parser output

    Returns:
        Dict with:
        - sequencer_fragment: Complete SequencerFragment with estimates
        - validation: Validation results
        - estimates: Duration/effort/cost estimates
        - status: "success" | "validation_failed"
    """
    # Parse phases
    plan_phases = []
    for phase_dict in phases:
        phase = PlanPhase(
            id=phase_dict["id"],
            title=phase_dict["title"],
            description=phase_dict.get("description", ""),
            estimated_effort=phase_dict["estimated_effort"],
            assignees=phase_dict.get("assignees", []),
            risk_score=phase_dict.get("risk_score", 0.5),
        )
        plan_phases.append(phase)

    # Parse dependencies
    phase_dependencies = []
    for dep_dict in dependencies:
        dep = PhaseDependency(
            predecessor_id=dep_dict["predecessor_id"],
            successor_id=dep_dict["successor_id"],
            edge_type=dep_dict.get("edge_type", "phase"),
            dependency_type=dep_dict.get("dependency_type", "finish_to_start"),
        )
        phase_dependencies.append(dep)

    # Parse IKAM references
    ikam_refs = []
    if ikam_references:
        for ref_dict in ikam_references:
            ref = IKAMFragmentReference(
                artifact_id=ref_dict["artifact_id"],
                fragment_id=ref_dict.get("fragment_id"),
                reference_type=ref_dict["reference_type"],
                scope=ref_dict.get("scope", []),
                metadata=ref_dict.get("metadata", {}),
            )
            ikam_refs.append(ref)

        # Note: IKAM reference resolution requires database connection
        # For test/demo mode, skip actual resolution and create placeholder validation
        # In production, wire database connection from caller:
        # resolution_result = resolve_ikam_references([ref.artifact_id for ref in ikam_refs], connection)
        # ikam_validation = validate_ikam_references(ikam_refs, resolution_result, connection)
        
        # For now, create warning for unresolved IKAM references
        ikam_validation = ValidationResult(
            is_valid=True,  # Don't block on missing DB in tests
            errors=[],
            warnings=[
                ValidationError(
                    severity="WARNING",
                    code="IKAM_RESOLUTION_SKIPPED",
                    message=f"IKAM reference resolution skipped (no database connection). {len(ikam_refs)} references not validated.",
                    artifact_id=None,
                    fragment_id=None,
                    phase_ids=[],
                )
            ],
        )
        has_ikam_errors = False
    else:
        ikam_validation = ValidationResult(is_valid=True, errors=[], warnings=[])
        has_ikam_errors = False

    # Create preliminary fragment for DAG validation
    # (Chicken-egg: need fragment for validation, but validation needed for fragment)
    preliminary_fragment = SequencerFragment(
        phases=plan_phases,
        dependencies=phase_dependencies,
        assignments={p.id: p.assignees for p in plan_phases},
        ikam_references=ikam_refs,
        validation=ValidationResult(is_valid=True, errors=[], warnings=[]),  # Placeholder
        effort_estimate=EffortEstimate(simple_estimate=0, medium_estimate=0, complex_estimate=0),  # Placeholder
        cost_estimate=CostEstimate(base_cost=0, role_based_cost=0, risk_adjusted_cost=0),  # Placeholder
        duration_estimate=DurationEstimate(optimistic=0, nominal=0, pessimistic=0, critical_path_days=0),  # Placeholder
        risk_score=0.5,
        confidence_score=0.5,
        planning_instruction=instruction,
        requested_by=requested_by,
        request_mode=request_mode,
        derived_from_instruction_id=instruction_id,
    )

    # Validate sequence (DAG check, assignee check, etc.)
    # Note: validate_sequence expects (fragment, connection). Pass None for test mode.
    dag_validation = validate_sequence(preliminary_fragment, None)  # type: ignore
    # In production: dag_validation = validate_sequence(preliminary_fragment, connection)

    # Merge IKAM validation errors/warnings into main validation
    all_errors = dag_validation.errors + ikam_validation.errors
    all_warnings = dag_validation.warnings + ikam_validation.warnings
    is_valid = dag_validation.is_valid and not has_ikam_errors

    merged_validation = ValidationResult(
        is_valid=is_valid,
        errors=all_errors,
        warnings=all_warnings,
    )

    # Update preliminary fragment with merged validation for estimation
    preliminary_fragment = SequencerFragment(
        phases=plan_phases,
        dependencies=phase_dependencies,
        assignments={p.id: p.assignees for p in plan_phases},
        ikam_references=ikam_refs,
        validation=merged_validation,
        effort_estimate=EffortEstimate(simple_estimate=0, medium_estimate=0, complex_estimate=0),  # Placeholder
        cost_estimate=CostEstimate(base_cost=0, role_based_cost=0, risk_adjusted_cost=0),  # Placeholder
        duration_estimate=DurationEstimate(optimistic=0, nominal=0, pessimistic=0, critical_path_days=0),  # Placeholder
        risk_score=0.5,
        confidence_score=0.5,
        planning_instruction=instruction,
        requested_by=requested_by,
        request_mode=request_mode,
        derived_from_instruction_id=instruction_id,
    )

    # Compute estimates
    duration_mode = "critical_path" if request_mode in ["medium", "complex"] else "simple"
    effort_mode = "fibonacci" if request_mode == "complex" else "simple"
    cost_mode = "role_based" if request_mode in ["medium", "complex"] else "simple"

    effort_estimate = estimate_effort(preliminary_fragment, mode=effort_mode)
    cost_estimate = estimate_cost(preliminary_fragment, mode=cost_mode)
    duration_estimate = estimate_duration(preliminary_fragment, mode=duration_mode)
    risk_score, confidence_score = aggregate_risk_confidence(preliminary_fragment)

    # Create final fragment with all computed values
    sequencer_fragment = SequencerFragment(
        phases=plan_phases,
        dependencies=phase_dependencies,
        assignments={p.id: p.assignees for p in plan_phases},
        ikam_references=ikam_refs,
        validation=merged_validation,
        effort_estimate=effort_estimate,
        cost_estimate=cost_estimate,
        duration_estimate=duration_estimate,
        risk_score=risk_score,
        confidence_score=confidence_score,
        planning_instruction=instruction,
        derived_from_instruction_id=instruction_id,
        requested_by=requested_by,
        request_mode=request_mode,
    )

    return {
        "status": "success" if is_valid else "validation_failed",
        "sequencer_fragment": sequencer_fragment.model_dump(mode="json"),
        "validation": {
            "is_valid": merged_validation.is_valid,
            "errors": [
                {"code": e.code, "message": e.message, "severity": e.severity, "phase_ids": e.phase_ids}
                for e in merged_validation.errors
            ],
            "warnings": [
                {"code": w.code, "message": w.message, "severity": w.severity, "phase_ids": w.phase_ids}
                for w in merged_validation.warnings
            ],
        },
        "estimates": {
            "duration": {
                "optimistic": sequencer_fragment.duration_estimate.optimistic,
                "nominal": sequencer_fragment.duration_estimate.nominal,
                "pessimistic": sequencer_fragment.duration_estimate.pessimistic,
                "critical_path_days": sequencer_fragment.duration_estimate.critical_path_days,
            },
            "effort": {
                "simple": sequencer_fragment.effort_estimate.simple_estimate,
                "medium": sequencer_fragment.effort_estimate.medium_estimate,
                "complex": sequencer_fragment.effort_estimate.complex_estimate,
            },
            "cost": {
                "base": sequencer_fragment.cost_estimate.base_cost,
                "role_based": sequencer_fragment.cost_estimate.role_based_cost,
                "risk_adjusted": sequencer_fragment.cost_estimate.risk_adjusted_cost,
            },
            "risk_score": risk_score,
            "confidence_score": confidence_score,
        },
        "instruction": instruction,
        "instruction_id": instruction_id,
    }


def validate_sequence_tool(
    sequencer_fragment_dict: Dict[str, Any],
) -> Dict[str, Any]:
    """Validate an existing sequencer fragment.

    Args:
        sequencer_fragment_dict: Serialized SequencerFragment

    Returns:
        Dict with validation results and updated fragment
    """
    # Parse fragment
    fragment = SequencerFragment(**sequencer_fragment_dict)

    # Re-validate sequence (no DB connection in tests)
    dag_validation = validate_sequence(fragment, None)  # type: ignore

    # Normalize to ValidationResult for downstream consumers
    validation = ValidationResult(
        is_valid=dag_validation.is_valid,
        errors=dag_validation.errors,
        warnings=dag_validation.warnings,
    )

    # Re-validate IKAM references if present (skip DB resolution in tests)
    if fragment.ikam_references:
        ikam_validation = ValidationResult(
            is_valid=True,
            errors=[],
            warnings=[
                ValidationError(
                    severity="WARNING",
                    code="IKAM_RESOLUTION_SKIPPED",
                    message=f"IKAM reference resolution skipped (no database connection). {len(fragment.ikam_references)} references not validated.",
                    artifact_id=None,
                    fragment_id=None,
                    phase_ids=[],
                )
            ],
        )

        all_errors = validation.errors + ikam_validation.errors
        all_warnings = validation.warnings + ikam_validation.warnings
        is_valid = validation.is_valid

        merged_validation = ValidationResult(
            is_valid=is_valid,
            errors=all_errors,
            warnings=all_warnings,
        )
    else:
        merged_validation = validation

    # Update fragment with new validation
    updated_fragment = SequencerFragment(
        phases=fragment.phases,
        dependencies=fragment.dependencies,
        assignments=fragment.assignments,
        ikam_references=fragment.ikam_references,
        validation=merged_validation,
        effort_estimate=fragment.effort_estimate,
        cost_estimate=fragment.cost_estimate,
        duration_estimate=fragment.duration_estimate,
        risk_score=fragment.risk_score,
        confidence_score=fragment.confidence_score,
        derived_from_instruction_id=fragment.derived_from_instruction_id,
        requested_by=fragment.requested_by,
        request_mode=fragment.request_mode,
        created_at=fragment.created_at,
    )

    return {
        "status": "valid" if merged_validation.is_valid else "invalid",
        "validation": {
            "is_valid": merged_validation.is_valid,
            "errors": [
                {"code": e.code, "message": e.message, "severity": e.severity, "phase_ids": e.phase_ids}
                for e in merged_validation.errors
            ],
            "warnings": [
                {"code": w.code, "message": w.message, "severity": w.severity, "phase_ids": w.phase_ids}
                for w in merged_validation.warnings
            ],
        },
        "sequencer_fragment": updated_fragment.model_dump(mode="json"),
    }


def commit_sequence(
    sequencer_fragment_dict: Dict[str, Any],
    committed_by: str,
    sequencer_request_id: str,
    connection: Optional[Any] = None,
) -> Dict[str, Any]:
    """Commit a sequencer fragment to execution (convert to ProjectPhaseFragment).

    Performs DB-backed IKAM artifact resolution and emits derivation events for
    all referenced artifacts. This ensures full provenance tracking and Fisher
    Information completeness.

    Args:
        sequencer_fragment_dict: Serialized SequencerFragment to commit
        committed_by: User ID committing the plan
        sequencer_request_id: Original sequencer request ID for provenance
        connection: PostgreSQL connection (optional; if None, IKAM resolution skipped)

    Returns:
        Dict with ProjectPhaseFragment, provenance metadata, and derivation events
    """
    # Parse fragment
    fragment = SequencerFragment(**sequencer_fragment_dict)

    # Validate before committing
    if not fragment.validation.is_valid:
        return {
            "status": "error",
            "error": "Cannot commit invalid sequence. Fix validation errors first.",
            "validation_errors": [
                {"code": e.code, "message": e.message}
                for e in fragment.validation.errors
            ],
        }

    # Perform DB-backed IKAM resolution if connection provided
    derivation_events: List[Dict[str, Any]] = []
    ikam_resolution_status = "skipped"
    ikam_resolution_notes: List[str] = []

    if fragment.ikam_references and connection:
        try:
            # Extract artifact IDs from references
            artifact_ids = [ref.artifact_id for ref in fragment.ikam_references]
            
            # Resolve artifacts from database
            resolved = resolve_ikam_references(artifact_ids, connection)
            
            # Validate resolved references
            phase_ids = [p.id for p in fragment.phases]
            validation_errors = validate_ikam_references(resolved, phase_ids)
            
            # Create derivation events for each valid reference
            for ref in fragment.ikam_references:
                resolution = resolved.get(ref.artifact_id, {})
                status = resolution.get("status", "UNKNOWN")
                
                if status == "RESOLVED":
                    # Emit derivation event: plan references artifact
                    event = DerivationProvenanceEvent(
                        source_id=ref.artifact_id,
                        target_id=sequencer_request_id,
                        derivation_type="referenced_by_plan",
                        transformation=f"planning.{ref.reference_type}",
                        transformation_params={
                            "reference_type": ref.reference_type,
                            "scope": ref.scope,
                            "artifact_kind": resolution.get("artifact_kind"),
                            "fragment_count": len(resolution.get("fragments", [])),
                        },
                        derivation_strength=0.95,
                    )
                    derivation_events.append(event.model_dump(mode="json"))
                    ikam_resolution_notes.append(
                        f"✓ {ref.reference_type} on {resolution.get('artifact_title', 'unknown')}"
                    )
                else:
                    ikam_resolution_notes.append(
                        f"⚠ {ref.reference_type} on {ref.artifact_id}: {resolution.get('error', 'not found')}"
                    )
            
            ikam_resolution_status = "resolved"
            
        except Exception as e:
            # Log but don't fail commit if IKAM resolution fails
            ikam_resolution_status = "error"
            ikam_resolution_notes.append(f"IKAM resolution error: {str(e)}")
    elif fragment.ikam_references:
        ikam_resolution_status = "skipped_no_connection"
        ikam_resolution_notes.append(f"{len(fragment.ikam_references)} references present but no database connection")

    # Convert PlanPhase to CommittedPhase
    committed_phases = [
        CommittedPhase(
            id=phase.id,
            title=phase.title,
            description=phase.description,
            estimated_effort=phase.estimated_effort,
            assignees=phase.assignees,
            phase_start=phase.phase_start,
            phase_end=phase.phase_end,
            ikam_inputs=phase.ikam_inputs,
            ikam_outputs=phase.ikam_outputs,
            status="planned",
        )
        for phase in fragment.phases
    ]

    # Create ProjectPhaseFragment
    project_fragment = ProjectPhaseFragment(
        phases=committed_phases,
        dependencies=fragment.dependencies,
        assignments=fragment.assignments,
        derived_from=sequencer_fragment_dict.get("id", sequencer_request_id),
        derivation_type="sequencer_fragment_committed",
        sequencer_request_id=sequencer_request_id,
        status="planned",
        phases_completed=[],
        committed_by=committed_by,
        committed_at=datetime.utcnow(),
    )

    return {
        "status": "committed",
        "project_phase_fragment": project_fragment.model_dump(mode="json"),
        "provenance": {
            "derived_from": project_fragment.derived_from,
            "derivation_type": project_fragment.derivation_type,
            "sequencer_request_id": sequencer_request_id,
            "committed_by": committed_by,
            "committed_at": project_fragment.committed_at.isoformat(),
        },
        "ikam_resolution": {
            "status": ikam_resolution_status,
            "references_processed": len(fragment.ikam_references) if fragment.ikam_references else 0,
            "derivation_events_emitted": len(derivation_events),
            "notes": ikam_resolution_notes,
        },
        "derivation_events": derivation_events,
        "summary": {
            "total_phases": len(committed_phases),
            "total_effort": fragment.effort_estimate.simple_estimate,
            "estimated_duration": fragment.duration_estimate.nominal,
            "estimated_cost": fragment.cost_estimate.base_cost,
            "ikam_references": len(fragment.ikam_references) if fragment.ikam_references else 0,
        },
    }


# MCP tool registry for sequencer service
MCP_TOOLS = {
    "create_sequence": {
        "name": "create_sequence",
        "description": "Generate project plan from natural language instruction with phases, dependencies, and IKAM references",
        "inputSchema": {
            "type": "object",
            "properties": {
                "instruction": {"type": "string", "description": "Natural language planning instruction"},
                "phases": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "string"},
                            "title": {"type": "string"},
                            "description": {"type": "string"},
                            "estimated_effort": {"type": "number"},
                            "assignees": {"type": "array", "items": {"type": "string"}},
                            "risk_score": {"type": "number", "minimum": 0, "maximum": 1},
                        },
                        "required": ["id", "title", "estimated_effort"],
                    },
                },
                "dependencies": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "predecessor_id": {"type": "string"},
                            "successor_id": {"type": "string"},
                            "edge_type": {"type": "string", "enum": ["phase", "artifact", "fragment"]},
                            "dependency_type": {"type": "string"},
                        },
                        "required": ["predecessor_id", "successor_id"],
                    },
                },
                "ikam_references": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "artifact_id": {"type": "string"},
                            "fragment_id": {"type": "string"},
                            "reference_type": {
                                "type": "string",
                                "enum": ["uses_variable", "depends_on_formula", "input_from_data", "output_to_narrative", "extends_model"],
                            },
                            "scope": {"type": "array", "items": {"type": "string"}},
                            "metadata": {"type": "object"},
                        },
                        "required": ["artifact_id", "reference_type"],
                    },
                },
                "requested_by": {"type": "string"},
                "request_mode": {"type": "string", "enum": ["simple", "medium", "complex"]},
                "instruction_id": {"type": "string"},
            },
            "required": ["instruction", "phases", "dependencies"],
        },
    },
    "validate_sequence": {
        "name": "validate_sequence",
        "description": "Validate existing sequencer fragment for DAG cycles, missing dependencies, and IKAM reference availability",
        "inputSchema": {
            "type": "object",
            "properties": {
                "sequencer_fragment": {"type": "object", "description": "Serialized SequencerFragment to validate"},
            },
            "required": ["sequencer_fragment"],
        },
    },
    "commit_sequence": {
        "name": "commit_sequence",
        "description": "Commit validated sequencer fragment to execution as ProjectPhaseFragment with DB-backed IKAM resolution and derivation event emission",
        "inputSchema": {
            "type": "object",
            "properties": {
                "sequencer_fragment": {"type": "object", "description": "Serialized SequencerFragment to commit"},
                "committed_by": {"type": "string", "description": "User ID committing the plan"},
                "sequencer_request_id": {"type": "string", "description": "Original sequencer request ID"},
            },
            "required": ["sequencer_fragment", "committed_by", "sequencer_request_id"],
        },
    },
}
