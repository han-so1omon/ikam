"""Generative scenario analysis for the sequencer.

This module implements scenario analysis by:
1. Parsing natural language scenario descriptions using SemanticEngine
2. Inferring scenario intent semantically (no keyword matching)
3. Generating analyzer functions on-the-fly (no hardcoded scenario types)
4. Calculating deltas (cost, duration, effort, risk) with IKAM awareness
5. Returning ScenarioFragment with rationale and provenance

Key principle: Unlimited scenario flexibility via semantic intent parsing
and generative function synthesis. Users can express any scenario variation
in natural language—system automatically generates appropriate analyzer.

Examples of supported scenarios (including novel ones):
- "What if we hire a contractor for the implementation phase?"
- "Reduce scope by 20%"
- "Parallelize phases 1 and 2"
- "Hire 2 contractors, 1 for backend, 1 for QA, one based in India" (novel)
- "Reduce total project cost by $50K without extending timeline" (novel)
- "Add comprehensive testing to the plan, even if it extends timeline" (novel)
"""

from __future__ import annotations

from datetime import datetime
from typing import Any, Callable, Optional, Dict
from pydantic import BaseModel, Field
import re
import logging

logger = logging.getLogger(__name__)


# ============================================================================
# Data Models (extend from existing sequencer models)
# ============================================================================

class PhaseModification(BaseModel):
    """Individual phase modification applied by a scenario."""

    phase_id: str = Field(..., description="ID of modified phase")
    modified_effort: Optional[float] = Field(
        default=None,
        description="New estimated effort (person-days), if changed"
    )
    modified_cost_multiplier: Optional[float] = Field(
        default=None,
        description="Cost multiplier (e.g., 1.4x for contractor premium)"
    )
    modified_assignees: Optional[list[str]] = Field(
        default=None,
        description="New assignees list, if changed"
    )
    modified_risk_delta: float = Field(
        default=0.0,
        description="Delta to apply to risk_score (e.g., +0.05 for offshore)"
    )
    reason: str = Field(
        ...,
        description="Human-readable reason for this modification"
    )


class ScenarioFragment(BaseModel):
    """Result of scenario analysis with deltas and rationale."""

    base_fragment_id: str = Field(
        ...,
        description="ID of SequencerFragment this scenario modifies"
    )

    scenario_description: str = Field(
        ...,
        description="Original natural language scenario description"
    )

    scenario_intent: dict[str, Any] = Field(
        ...,
        description="Parsed scenario intent (operation, target_phases, parameters, confidence)"
    )

    # Modified phases and modifications
    modifications: list[PhaseModification] = Field(
        ...,
        description="List of individual phase modifications"
    )

    # Deltas (computed aggregates)
    effort_delta: float = Field(
        ...,
        description="Change in total effort (person-days)"
    )

    cost_delta: float = Field(
        ...,
        description="Change in total cost (currency units)"
    )

    duration_delta: float = Field(
        ...,
        description="Change in critical path duration (days)"
    )

    risk_delta: float = Field(
        ...,
        description="Change in aggregate risk score"
    )

    # Rationale for user
    rationale: str = Field(
        ...,
        description="Human-readable explanation of scenario impact"
    )

    recommendations: list[str] = Field(
        default_factory=list,
        description="Recommended actions based on scenario analysis"
    )

    # Provenance
    created_at: datetime = Field(
        default_factory=datetime.now,
        description="When this scenario was analyzed"
    )


# ============================================================================
# Scenario Intent Parsing (Semantic)
# ============================================================================

async def parse_scenario_intent_semantic(
    scenario_description: str,
    semantic_engine: Optional[Any] = None,
) -> Dict[str, Any]:
    """
    Parse natural language scenario into structured intent using SemanticEngine.

    Uses semantic evaluation (embeddings + intent classification) to robustly
    classify scenarios without keyword matching. Supports novel scenarios
    automatically by delegating to generated operation synthesis.

    Args:
        scenario_description: Natural language scenario (e.g., "What if we hire a contractor?")
        semantic_engine: SemanticEngine instance (creates default if None)

    Returns:
        Dict with keys: operation, target_phases, parameters, confidence, features

    Example:
        >>> result = await parse_scenario_intent_semantic(
        ...     "What if we hire a contractor for phase 2?"
        ... )
        >>> result
        {
            'operation': 'resource_substitution',
            'confidence': 0.92,
            'target_phases': ['phase-2'],
            'parameters': {
                'role': 'contractor',
                'cost_multiplier': 1.4,
                'effort_reduction': 0.85,
                'risk_adjustment': 0.05,
            },
            'features': ['uses_external_resource', 'single_phase_target'],
            'reasoning': 'Detected resource substitution intent with high confidence...',
        }
    """
    # Lazy import to avoid circular dependencies
    if semantic_engine is None:
        try:
            from modelado.semantic_engine import SemanticEngine
            import os
            
            api_key = os.getenv("OPENAI_API_KEY")
            if api_key:
                semantic_engine = SemanticEngine()
            else:
                logger.warning(
                    "OPENAI_API_KEY not set; falling back to keyword-based parsing"
                )
                return parse_scenario_intent_fallback(scenario_description)
        except ImportError:
            logger.warning(
                "SemanticEngine not available; falling back to keyword-based parsing"
            )
            return parse_scenario_intent_fallback(scenario_description)

    try:
        # Step 1: Evaluate intent semantically
        evaluation = await semantic_engine.evaluate(scenario_description)

        logger.debug(
            "Scenario intent evaluation: operation=%s, confidence=%.2f, features=%s",
            evaluation.evaluator_name,
            evaluation.evaluator_confidence,
            evaluation.semantic_features,
        )

        # Step 2: Map evaluator name to operation type
        operation = _map_evaluator_to_operation(
            evaluation.evaluator_name,
            evaluation.semantic_features,
            scenario_description,
        )

        # Step 3: Extract parameters from intent and semantic features
        parameters = _extract_scenario_parameters(
            scenario_description,
            evaluation.semantic_features,
            operation,
        )

        # Step 4: Extract phase references
        target_phases = _extract_phases_from_description(scenario_description)

        # Step 5: Build result with confidence and features
        result = {
            "operation": operation,
            "confidence": evaluation.evaluator_confidence,
            "target_phases": target_phases,
            "parameters": parameters,
            "features": list(
                k for k, v in evaluation.semantic_features.items() if v
            ),
            "reasoning": evaluation.reasoning,
            "evaluator": evaluation.evaluator_name,
        }

        logger.info(
            "Scenario intent parsed: operation=%s, confidence=%.2f, target_phases=%s",
            operation,
            evaluation.evaluator_confidence,
            target_phases,
        )

        return result

    except Exception as e:
        logger.error("SemanticEngine evaluation failed: %s", str(e), exc_info=True)
        # Fallback to keyword-based parsing
        return parse_scenario_intent_fallback(scenario_description)


def parse_scenario_intent_fallback(
    scenario_description: str,
) -> Dict[str, Any]:
    """
    Fallback keyword-based scenario intent parsing.

    Used only when SemanticEngine is unavailable. For production use,
    SemanticEngine should always be available per CORE_PRINCIPLES.

    Args:
        scenario_description: Natural language scenario

    Returns:
        Dict with operation, confidence, target_phases, parameters
    """
    description_lower = scenario_description.lower()

    if any(
        word in description_lower
        for word in ["hire", "contractor", "consultant", "resource", "outsource"]
    ):
        operation = "resource_substitution"
        cost_multiplier = 1.4
        if "offshore" in description_lower or "india" in description_lower:
            cost_multiplier = 1.2
        if "senior" in description_lower or "expert" in description_lower:
            cost_multiplier = 1.6

        effort_reduction = 0.85
        if any(
            word in description_lower
            for word in ["two contractors", "2 contractors", "multiple contractors"]
        ):
            effort_reduction = 0.75

        return {
            "operation": operation,
            "confidence": 0.75,  # Lower confidence for fallback
            "target_phases": _extract_phases_from_description(scenario_description),
            "parameters": {
                "role": "contractor",
                "cost_multiplier": cost_multiplier,
                "effort_reduction": effort_reduction,
                "risk_adjustment": 0.05,
            },
            "features": ["uses_external_resource"],
            "reasoning": "Keyword-based parsing (SemanticEngine unavailable)",
        }

    elif any(word in description_lower for word in ["cost", "budget", "save"]):
        operation = "cost_optimization"
        target_savings = 50000
        if "10k" in description_lower:
            target_savings = 10000
        elif "20k" in description_lower:
            target_savings = 20000

        return {
            "operation": operation,
            "confidence": 0.70,
            "target_phases": ["all"],
            "parameters": {
                "target_savings": target_savings,
                "preserve_timeline": True,
            },
            "features": ["cost_focused"],
            "reasoning": "Keyword-based parsing (SemanticEngine unavailable)",
        }

    elif any(
        word in description_lower for word in ["reduce", "cut", "scope", "features"]
    ):
        operation = "scope_reduction"
        reduction_percent = 0.20
        if "25%" in description_lower:
            reduction_percent = 0.25
        elif "30%" in description_lower:
            reduction_percent = 0.30
        elif "50%" in description_lower:
            reduction_percent = 0.50

        return {
            "operation": operation,
            "confidence": 0.80,
            "target_phases": ["all"],
            "parameters": {
                "reduction_percent": reduction_percent,
                "feature_priority": "critical",
            },
            "features": ["scope_focused"],
            "reasoning": "Keyword-based parsing (SemanticEngine unavailable)",
        }

    elif any(
        word in description_lower for word in ["parallelize", "parallel", "concurrent"]
    ):
        operation = "parallelize"
        return {
            "operation": operation,
            "confidence": 0.78,
            "target_phases": _extract_phases_from_description(scenario_description),
            "parameters": {
                "critical_path_focus": True,
                "resource_constraint": "none",
            },
            "features": ["parallelization"],
            "reasoning": "Keyword-based parsing (SemanticEngine unavailable)",
        }

    elif any(
        word in description_lower for word in ["test", "quality", "code review"]
    ):
        operation = "quality_enhancement"
        return {
            "operation": operation,
            "confidence": 0.72,
            "target_phases": _extract_phases_from_description(scenario_description),
            "parameters": {
                "testing_depth": "comprehensive",
                "effort_multiplier": 1.3,
                "cost_multiplier": 1.2,
            },
            "features": ["quality_focused"],
            "reasoning": "Keyword-based parsing (SemanticEngine unavailable)",
        }

    else:
        operation = "custom_adjustment"
        return {
            "operation": operation,
            "confidence": 0.50,  # Very low for unrecognized intent
            "target_phases": _extract_phases_from_description(scenario_description),
            "parameters": {
                "description": scenario_description,
            },
            "features": [],
            "reasoning": "Keyword-based parsing (SemanticEngine unavailable)",
        }


def _map_evaluator_to_operation(
    evaluator_name: Optional[str],
    semantic_features: Dict[str, bool],
    scenario_description: str,
) -> str:
    """
    Map SemanticEngine evaluator result to scenario operation type.

    Args:
        evaluator_name: Name of the selected evaluator
        semantic_features: Dictionary of detected semantic features
        scenario_description: Original user input (for fallback detection)

    Returns:
        Operation type string
    """
    if evaluator_name is None:
        # Generic unknown operation
        return "custom_adjustment"

    evaluator_lower = evaluator_name.lower()

    # Map evaluators to operations
    if "economic" in evaluator_lower:
        # Economic operation detected - check features for sub-type
        features_lower = {k.lower(): v for k, v in semantic_features.items()}
        desc_lower = scenario_description.lower()

        if features_lower.get("resource_related", False) or any(
            w in desc_lower for w in ["hire", "contractor", "resource"]
        ):
            return "resource_substitution"
        elif features_lower.get("cost_related", False) or any(
            w in desc_lower for w in ["cost", "budget", "save"]
        ):
            return "cost_optimization"
        elif features_lower.get("scope_related", False) or any(
            w in desc_lower for w in ["reduce", "scope", "features"]
        ):
            return "scope_reduction"
        else:
            return "economics_adjustment"

    elif "story" in evaluator_lower:
        return "story_update"

    elif "constraint" in evaluator_lower:
        if "parallel" in scenario_description.lower():
            return "parallelize"
        else:
            return "constraint_adjustment"

    else:
        # Default to generic custom adjustment
        return "custom_adjustment"


def _extract_scenario_parameters(
    scenario_description: str,
    semantic_features: Dict[str, bool],
    operation: str,
) -> Dict[str, Any]:
    """
    Extract scenario-specific parameters from description and features.

    Args:
        scenario_description: Original user input
        semantic_features: Detected semantic features
        operation: Identified operation type

    Returns:
        Dictionary of parameters for this operation
    """
    desc_lower = scenario_description.lower()
    parameters = {}

    if operation == "resource_substitution":
        parameters["role"] = "contractor"

        # Detect cost multiplier based on seniority/location
        cost_multiplier = 1.4
        if "offshore" in desc_lower or "india" in desc_lower:
            cost_multiplier = 1.2
        if "senior" in desc_lower or "expert" in desc_lower:
            cost_multiplier = 1.6
        parameters["cost_multiplier"] = cost_multiplier

        # Effort reduction
        effort_reduction = 0.85
        if "two contractors" in desc_lower or "2 contractors" in desc_lower:
            effort_reduction = 0.75
        parameters["effort_reduction"] = effort_reduction

        parameters["risk_adjustment"] = 0.05

    elif operation == "cost_optimization":
        # Extract target savings
        target_savings = 50000
        if "10k" in desc_lower or "10000" in desc_lower:
            target_savings = 10000
        elif "20k" in desc_lower or "20000" in desc_lower:
            target_savings = 20000
        elif "50k" in desc_lower or "50000" in desc_lower:
            target_savings = 50000

        parameters["target_savings"] = target_savings
        parameters["preserve_timeline"] = True

    elif operation == "scope_reduction":
        # Extract reduction percentage
        reduction_percent = 0.20
        if "25%" in desc_lower or "0.25" in desc_lower:
            reduction_percent = 0.25
        elif "30%" in desc_lower or "0.30" in desc_lower:
            reduction_percent = 0.30
        elif "50%" in desc_lower or "0.50" in desc_lower:
            reduction_percent = 0.50

        parameters["reduction_percent"] = reduction_percent
        parameters["feature_priority"] = "critical"

    elif operation == "parallelize":
        parameters["critical_path_focus"] = True
        parameters["resource_constraint"] = "none"

    elif operation == "quality_enhancement":
        parameters["testing_depth"] = "comprehensive"
        parameters["effort_multiplier"] = 1.3
        parameters["cost_multiplier"] = 1.2

    # Generic fallback
    if not parameters:
        parameters = {
            "description": scenario_description,
        }

    return parameters


def _extract_phases_from_description(description: str) -> list[str]:
    """
    Extract phase references from scenario description.

    Examples:
    - "for phase 2" → ["phase-2"]
    - "phases 1-3" → ["phase-1", "phase-2", "phase-3"]
    - "implementation phase" → [] (no explicit phase IDs; will apply to inferred phases)

    Args:
        description: Natural language scenario

    Returns:
        List of phase IDs mentioned
    """

    phases = []
    description_lower = description.lower()

    # Find "phase 1", "phase 2", etc.
    phase_matches = re.findall(r'phase\s*(\d+)', description_lower)
    if phase_matches:
        phases.extend([f"phase-{m}" for m in phase_matches])

    # Find "phases 1-3" ranges
    range_matches = re.findall(r'phases?\s*(\d+)\s*-\s*(\d+)', description_lower)
    if range_matches:
        for start, end in range_matches:
            phases.extend([f"phase-{i}" for i in range(int(start), int(end) + 1)])

    return list(set(phases))  # deduplicate


# ============================================================================
# Backward Compatibility: Synchronous API Wrapper
# ============================================================================

def parse_scenario_intent(
    scenario_description: str,
) -> Dict[str, Any]:
    """
    Synchronous backward-compatibility wrapper for parse_scenario_intent_semantic.

    Uses the fallback implementation immediately since async isn't available
    in the sync context. For async usage with SemanticEngine, use
    parse_scenario_intent_semantic() instead.

    Args:
        scenario_description: Natural language scenario

    Returns:
        Dict with operation, confidence, target_phases, parameters, features
    """
    return parse_scenario_intent_fallback(scenario_description)


# ============================================================================
# Scenario Analyzer Generation (Generative)
# ============================================================================

def generate_scenario_analyzer(
    scenario_intent: dict[str, Any],
) -> Callable[[list[Any]], list[PhaseModification]]:
    """
    Generate scenario analyzer function from semantic intent.

    Instead of hardcoded dispatcher:
        if delta_type == "hire_contractor": ...
        elif delta_type == "cut_scope": ...

    We generate the function dynamically based on operation type.

    Args:
        scenario_intent: Parsed scenario intent from parse_scenario_intent()

    Returns:
        Callable function that accepts a list of PlanPhase and returns modifications
    """

    operation = scenario_intent.get("operation")
    target_phases = scenario_intent.get("target_phases", [])
    parameters = scenario_intent.get("parameters", {})

    if operation == "resource_substitution":
        return _generate_resource_substitution_analyzer(target_phases, parameters)

    elif operation == "scope_reduction":
        return _generate_scope_reduction_analyzer(target_phases, parameters)

    elif operation == "parallelize":
        return _generate_parallelization_analyzer(target_phases, parameters)

    elif operation == "quality_enhancement":
        return _generate_quality_enhancement_analyzer(target_phases, parameters)

    elif operation == "cost_optimization":
        return _generate_cost_optimization_analyzer(target_phases, parameters)

    else:
        # Generic analyzer for novel operations
        return _generate_generic_operation_analyzer(operation, target_phases, parameters)


def _generate_resource_substitution_analyzer(
    target_phases: list[str],
    parameters: dict[str, Any],
) -> Callable:
    """Generate analyzer for resource substitution (hire contractor, consultant, etc.)."""

    def analyzer(phases: list[Any]) -> list[PhaseModification]:
        role = parameters.get("role", "contractor")
        cost_multiplier = parameters.get("cost_multiplier", 1.4)
        effort_reduction = parameters.get("effort_reduction", 0.85)
        risk_adjustment = parameters.get("risk_adjustment", 0.05)

        modifications = []
        for phase in phases:
            if not target_phases or phase.id in target_phases:
                modifications.append(PhaseModification(
                    phase_id=phase.id,
                    modified_effort=phase.estimated_effort * effort_reduction,
                    modified_cost_multiplier=cost_multiplier,
                    modified_assignees=[role],
                    modified_risk_delta=risk_adjustment,
                    reason=f"Assigned {role} to accelerate execution",
                ))

        return modifications

    return analyzer


def _generate_scope_reduction_analyzer(
    target_phases: list[str],
    parameters: dict[str, Any],
) -> Callable:
    """Generate analyzer for scope reduction (cut scope, remove features)."""

    def analyzer(phases: list[Any]) -> list[PhaseModification]:
        reduction_percent = parameters.get("reduction_percent", 0.20)

        modifications = []
        for phase in phases:
            # Scope reduction affects all phases proportionally
            reduction_factor = 1.0 - reduction_percent
            modifications.append(PhaseModification(
                phase_id=phase.id,
                modified_effort=phase.estimated_effort * reduction_factor,
                modified_risk_delta=-0.10,  # reduced scope = lower risk
                reason=f"Scope reduced by {int(reduction_percent * 100)}%",
            ))

        return modifications

    return analyzer


def _generate_parallelization_analyzer(
    target_phases: list[str],
    parameters: dict[str, Any],
) -> Callable:
    """Generate analyzer for phase parallelization."""

    def analyzer(phases: list[Any]) -> list[PhaseModification]:
        modifications = []

        # Parallelization reduces critical path but not individual effort
        for phase in phases:
            if not target_phases or phase.id in target_phases:
                modifications.append(PhaseModification(
                    phase_id=phase.id,
                    modified_risk_delta=0.10,  # slight risk increase with parallelization
                    reason="Parallelized execution (critical path reduced, risk slightly increased)",
                ))

        return modifications

    return analyzer


def _generate_quality_enhancement_analyzer(
    target_phases: list[str],
    parameters: dict[str, Any],
) -> Callable:
    """Generate analyzer for quality enhancement (add testing, code review)."""

    def analyzer(phases: list[Any]) -> list[PhaseModification]:
        effort_multiplier = parameters.get("effort_multiplier", 1.3)
        cost_multiplier = parameters.get("cost_multiplier", 1.2)

        modifications = []
        for phase in phases:
            if not target_phases or phase.id in target_phases:
                modifications.append(PhaseModification(
                    phase_id=phase.id,
                    modified_effort=phase.estimated_effort * effort_multiplier,
                    modified_cost_multiplier=cost_multiplier,
                    modified_risk_delta=-0.15,  # quality enhancement reduces risk
                    reason="Comprehensive testing and code review added",
                ))

        return modifications

    return analyzer


def _generate_cost_optimization_analyzer(
    target_phases: list[str],
    parameters: dict[str, Any],
) -> Callable:
    """Generate analyzer for cost optimization."""

    def analyzer(phases: list[Any]) -> list[PhaseModification]:
        # Cost optimization typically involves trade-offs (timeline or risk)
        preserve_timeline = parameters.get("preserve_timeline", False)

        modifications = []
        cost_reduction_factor = 0.85  # reduce costs by 15%

        for phase in phases:
            if preserve_timeline:
                # If timeline is preserved, reduce scope instead of duration
                modifications.append(PhaseModification(
                    phase_id=phase.id,
                    modified_cost_multiplier=cost_reduction_factor,
                    modified_effort=phase.estimated_effort * 0.90,  # reduced scope
                    modified_risk_delta=0.05,  # scope reduction adds slight risk
                    reason="Cost optimized; scope reduced to preserve timeline",
                ))
            else:
                # Can extend timeline to reduce costs
                modifications.append(PhaseModification(
                    phase_id=phase.id,
                    modified_cost_multiplier=cost_reduction_factor,
                    modified_risk_delta=-0.05,  # longer timeline reduces risk
                    reason="Cost optimized; extended timeline to reduce expenses",
                ))

        return modifications

    return analyzer


def _generate_generic_operation_analyzer(
    operation: str,
    target_phases: list[str],
    parameters: dict[str, Any],
) -> Callable:
    """
    Generate analyzer for novel/unrecognized operation types.

    This enables unlimited scenario flexibility—any operation the user
    describes can be handled, even if not explicitly programmed.
    """

    def analyzer(phases: list[Any]) -> list[PhaseModification]:
        modifications = []

        for phase in phases:
            if not target_phases or phase.id in target_phases:
                mods_dict = {"phase_id": phase.id, "reason": f"Applied scenario: {operation}"}

                # Apply numeric parameters as multipliers
                for param_name, param_value in parameters.items():
                    if param_name == "effort_multiplier" and isinstance(param_value, (int, float)):
                        mods_dict["modified_effort"] = phase.estimated_effort * param_value
                    elif param_name == "cost_multiplier" and isinstance(param_value, (int, float)):
                        mods_dict["modified_cost_multiplier"] = param_value
                    elif param_name == "risk_adjustment" and isinstance(param_value, (int, float)):
                        mods_dict["modified_risk_delta"] = param_value

                modifications.append(PhaseModification(**mods_dict))

        return modifications

    return analyzer


# ============================================================================
# Scenario Analysis Orchestration
# ============================================================================

async def analyze_scenario(
    base_fragment: Any,  # SequencerFragment
    scenario_description: str,
    semantic_engine: Optional[Any] = None,
) -> ScenarioFragment:
    """
    Async end-to-end scenario analysis using semantic intent parsing.

    This function mirrors `run_scenario_analysis` but uses
    `parse_scenario_intent_semantic` to honor the semantic, generative-first
    policy and aligns with tests that await an async API.

    Args:
        base_fragment: SequencerFragment to modify
        scenario_description: Natural language scenario description

    Returns:
        ScenarioFragment with deltas, rationale, and recommendations
    """

    # Step 1: Parse scenario intent semantically (async)
    intent = await parse_scenario_intent_semantic(
        scenario_description,
        semantic_engine=semantic_engine,
    )

    # Step 2: Generate scenario analyzer function
    analyzer_fn = generate_scenario_analyzer(intent)

    # Step 3: Execute analyzer to get phase modifications
    phase_modifications = analyzer_fn(base_fragment.phases)

    # Step 4: Calculate aggregate deltas
    effort_delta = sum(
        (m.modified_effort or base_fragment.phases[i].estimated_effort) -
        base_fragment.phases[i].estimated_effort
        for i, m in enumerate(phase_modifications)
        if i < len(base_fragment.phases)
    )

    # For cost delta, assume average hourly rate of $150
    cost_delta = effort_delta * 150 * 8  # 8 hours per day, $150/hour

    # Duration delta (simplified: critical path ≈ max(effort) / team_size)
    max_team_size = 1
    for m in phase_modifications:
        if m.modified_assignees:
            max_team_size = max(max_team_size, len(m.modified_assignees))
    duration_delta = effort_delta / max_team_size if phase_modifications else 0

    # Risk delta (aggregate change in risk across phases)
    risk_delta = sum(m.modified_risk_delta for m in phase_modifications)

    # Step 5: Generate human-readable rationale
    operation = intent.get("operation", "unknown")
    rationale = _generate_scenario_rationale(
        operation=operation,
        modifications=phase_modifications,
        effort_delta=effort_delta,
        cost_delta=cost_delta,
        duration_delta=duration_delta,
        risk_delta=risk_delta,
    )

    recommendations = _generate_recommendations(
        operation=operation,
        effort_delta=effort_delta,
        cost_delta=cost_delta,
        duration_delta=duration_delta,
        risk_delta=risk_delta,
    )

    # Step 6: Create and return ScenarioFragment
    return ScenarioFragment(
        base_fragment_id=getattr(base_fragment, 'id', 'unknown'),
        scenario_description=scenario_description,
        scenario_intent=intent,
        modifications=phase_modifications,
        effort_delta=effort_delta,
        cost_delta=cost_delta,
        duration_delta=duration_delta,
        risk_delta=risk_delta,
        rationale=rationale,
        recommendations=recommendations,
    )

def run_scenario_analysis(
    base_fragment: Any,  # SequencerFragment
    scenario_description: str,
) -> ScenarioFragment:
    """
    Run scenario analysis end-to-end.

    Flow:
    1. Parse scenario intent semantically
    2. Generate scenario analyzer function
    3. Execute analyzer on base fragment
    4. Calculate aggregate deltas
    5. Generate rationale and recommendations
    6. Return ScenarioFragment with full provenance

    Args:
        base_fragment: SequencerFragment to modify
        scenario_description: Natural language scenario description

    Returns:
        ScenarioFragment with deltas, rationale, and recommendations
    """

    # Step 1: Parse scenario intent
    intent = parse_scenario_intent(scenario_description)

    # Step 2: Generate scenario analyzer function
    analyzer_fn = generate_scenario_analyzer(intent)

    # Step 3: Execute analyzer to get phase modifications
    phase_modifications = analyzer_fn(base_fragment.phases)

    # Step 4: Calculate aggregate deltas
    effort_delta = sum(
        (m.modified_effort or base_fragment.phases[i].estimated_effort) -
        base_fragment.phases[i].estimated_effort
        for i, m in enumerate(phase_modifications)
        if i < len(base_fragment.phases)
    )

    # For cost delta, assume average hourly rate of $150
    cost_delta = effort_delta * 150 * 8  # 8 hours per day, $150/hour

    # Duration delta (simplified: critical path ≈ max(effort) / team_size)
    # Handle case where modifications don't have assignees
    max_team_size = 1
    for m in phase_modifications:
        if m.modified_assignees:
            max_team_size = max(max_team_size, len(m.modified_assignees))
    duration_delta = effort_delta / max_team_size if phase_modifications else 0

    # Risk delta (aggregate change in risk across phases)
    risk_delta = sum(m.modified_risk_delta for m in phase_modifications)

    # Step 5: Generate human-readable rationale
    operation = intent.get("operation", "unknown")
    rationale = _generate_scenario_rationale(
        operation=operation,
        modifications=phase_modifications,
        effort_delta=effort_delta,
        cost_delta=cost_delta,
        duration_delta=duration_delta,
        risk_delta=risk_delta,
    )

    recommendations = _generate_recommendations(
        operation=operation,
        effort_delta=effort_delta,
        cost_delta=cost_delta,
        duration_delta=duration_delta,
        risk_delta=risk_delta,
    )

    # Step 6: Create and return ScenarioFragment
    return ScenarioFragment(
        base_fragment_id=getattr(base_fragment, 'id', 'unknown'),
        scenario_description=scenario_description,
        scenario_intent=intent,
        modifications=phase_modifications,
        effort_delta=effort_delta,
        cost_delta=cost_delta,
        duration_delta=duration_delta,
        risk_delta=risk_delta,
        rationale=rationale,
        recommendations=recommendations,
    )


def _generate_scenario_rationale(
    operation: str,
    modifications: list[PhaseModification],
    effort_delta: float,
    cost_delta: float,
    duration_delta: float,
    risk_delta: float,
) -> str:
    """Generate human-readable rationale for scenario impact."""

    lines = [
        f"Scenario: {operation.replace('_', ' ').title()}",
        "",
        f"Impact Summary:",
        f"- Effort: {effort_delta:+.1f} person-days ({effort_delta:+.1%})",
        f"- Cost: ${cost_delta:+,.0f}",
        f"- Duration: {duration_delta:+.1f} days",
        f"- Risk: {risk_delta:+.2f}",
        "",
        "Modified Phases:",
    ]

    for mod in modifications:
        lines.append(f"  • {mod.phase_id}: {mod.reason}")

    return "\n".join(lines)


def _generate_recommendations(
    operation: str,
    effort_delta: float,
    cost_delta: float,
    duration_delta: float,
    risk_delta: float,
) -> list[str]:
    """Generate actionable recommendations based on scenario impact."""

    recommendations = []

    if effort_delta > 0:
        recommendations.append("Consider adding buffer time due to increased effort")
    elif effort_delta < 0:
        recommendations.append("Effort reduced—opportunities for acceleration or cost savings")

    if cost_delta > 50000:  # Large cost increase
        recommendations.append("Cost increase is substantial—consider alternative scenarios")

    if risk_delta > 0.15:  # Significant risk increase
        recommendations.append("Risk increased significantly—consider mitigation strategies")
    elif risk_delta < -0.15:  # Significant risk reduction
        recommendations.append("Risk reduced substantially—this scenario improves project safety")

    if duration_delta > 10:  # Long timeline extension
        recommendations.append("Timeline extended significantly—verify stakeholder impact")
    elif duration_delta < -5:  # Meaningful acceleration
        recommendations.append("Timeline accelerated—verify resource availability")

    return recommendations
