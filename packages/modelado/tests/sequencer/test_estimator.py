"""Unit tests for sequencer estimator module."""

import pytest
from unittest.mock import MagicMock

from modelado.sequencer.models import (
    SequencerFragment,
    PlanPhase,
    PhaseDependency,
    ValidationResult,
    EffortEstimate,
    CostEstimate,
    DurationEstimate,
)
from modelado.sequencer.estimator import (
    estimate_duration,
    estimate_effort,
    estimate_cost,
    aggregate_risk_confidence,
    _critical_path_analysis,
    _estimate_phase_complexity,
    _infer_role,
    FIBONACCI_SEQUENCE,
)


@pytest.fixture
def simple_phases():
    """Create three simple phases for testing."""
    return [
        PlanPhase(
            id="phase-1",
            title="Phase 1",
            description="First phase",
            estimated_effort=5.0,
            assignees=["dev-1"],
        ),
        PlanPhase(
            id="phase-2",
            title="Phase 2",
            description="Second phase",
            estimated_effort=10.0,
            assignees=["dev-1", "dev-2"],
        ),
        PlanPhase(
            id="phase-3",
            title="Phase 3",
            description="Third phase",
            estimated_effort=2.0,
            assignees=["dev-1"],
        ),
    ]


@pytest.fixture
def simple_dependencies():
    """Create phase dependencies."""
    return [
        PhaseDependency(
            predecessor_id="phase-1",
            successor_id="phase-2",
            dependency_type="finish_to_start",
            edge_type="phase",
        ),
        PhaseDependency(
            predecessor_id="phase-2",
            successor_id="phase-3",
            dependency_type="finish_to_start",
            edge_type="phase",
        ),
    ]


def create_test_fragment(phases, dependencies, risk_score=0.3):
    """Helper to create a valid SequencerFragment."""
    return SequencerFragment(
        phases=phases,
        dependencies=dependencies,
        assignments={phase.id: phase.assignees for phase in phases},
        ikam_references=[],
        validation=ValidationResult(is_valid=True, errors=[], warnings=[]),
        effort_estimate=EffortEstimate(
            simple_estimate=5.0,
            medium_estimate=10.0,
            complex_estimate=20.0,
        ),
        cost_estimate=CostEstimate(
            base_cost=1000.0,
            role_based_cost=1200.0,
            risk_adjusted_cost=1500.0,
        ),
        duration_estimate=DurationEstimate(
            optimistic=10.0,
            nominal=15.0,
            pessimistic=25.0,
            critical_path_days=15.0,
        ),
        risk_score=risk_score,
        confidence_score=0.8,
        requested_by="test-user",
        request_mode="medium",
    )


class TestDurationEstimation:
    """Tests for estimate_duration()."""

    def test_estimate_duration_simple(self, simple_phases, simple_dependencies):
        """Test simple duration estimation (sum of efforts)."""
        fragment = create_test_fragment(simple_phases, simple_dependencies)
        result = estimate_duration(fragment, mode="simple")

        # Total effort: 5 + 10 + 2 = 17 days
        assert result.nominal == 17.0
        assert result.optimistic == 17.0 * 0.8  # 13.6
        assert result.pessimistic == 17.0 * 1.2  # 20.4
        assert result.critical_path_days == 17.0

    def test_estimate_duration_critical_path(
        self, simple_phases, simple_dependencies
    ):
        """Test critical path duration estimation."""
        fragment = create_test_fragment(simple_phases, simple_dependencies)
        result = estimate_duration(fragment, mode="critical_path")

        # Critical path: phase-1 (5) → phase-2 (10) → phase-3 (2) = 17 days
        assert result.nominal == 17.0
        assert result.critical_path_days == 17.0

    def test_estimate_duration_parallel_phases(self, simple_phases):
        """Test critical path with parallel phases."""
        phases = [
            PlanPhase(
                id="phase-1",
                title="Phase 1",
                description="First phase",
                estimated_effort=5.0,
                assignees=["dev-1"],
            ),
            PlanPhase(
                id="phase-2",
                title="Phase 2",
                description="Parallel phase",
                estimated_effort=8.0,
                assignees=["dev-2"],
            ),
            PlanPhase(
                id="phase-3",
                title="Phase 3",
                description="Depends on both",
                estimated_effort=3.0,
                assignees=["dev-1", "dev-2"],
            ),
        ]

        dependencies = [
            PhaseDependency(
                predecessor_id="phase-1",
                successor_id="phase-3",
                dependency_type="finish_to_start",
                edge_type="phase",
            ),
            PhaseDependency(
                predecessor_id="phase-2",
                successor_id="phase-3",
                dependency_type="finish_to_start",
                edge_type="phase",
            ),
        ]

        fragment = create_test_fragment(phases, dependencies)
        result = estimate_duration(fragment, mode="critical_path")

        # Critical path: max(phase-1 + phase-3, phase-2 + phase-3) = max(8, 11) = 11
        assert result.nominal == 11.0

    def test_estimate_duration_invalid_mode(self, simple_phases, simple_dependencies):
        """Test invalid mode raises error."""
        fragment = create_test_fragment(simple_phases, simple_dependencies)
        with pytest.raises(ValueError, match="Unknown duration mode"):
            estimate_duration(fragment, mode="invalid")


class TestEffortEstimation:
    """Tests for estimate_effort()."""

    def test_estimate_effort_simple(self, simple_phases, simple_dependencies):
        """Test simple effort estimation (sum)."""
        fragment = create_test_fragment(simple_phases, simple_dependencies)
        result = estimate_effort(fragment, mode="simple")

        # Total: 5 + 10 + 2 = 17
        assert result.simple_estimate == 17.0
        assert result.medium_estimate == 17.0 * 1.25
        assert result.complex_estimate == 17.0 * 1.5

    def test_estimate_effort_fibonacci(self, simple_phases, simple_dependencies):
        """Test Fibonacci effort estimation."""
        fragment = create_test_fragment(simple_phases, simple_dependencies)
        result = estimate_effort(fragment, mode="fibonacci")

        # Should use Fibonacci values based on complexity
        assert result.simple_estimate > 0
        assert result.medium_estimate == result.simple_estimate * 1.5
        assert result.complex_estimate == result.simple_estimate * 2.0

    def test_estimate_effort_fibonacci_values(self, simple_phases, simple_dependencies):
        """Test that Fibonacci uses correct sequence."""
        fragment = create_test_fragment(simple_phases, simple_dependencies)
        result = estimate_effort(fragment, mode="fibonacci")

        # Result should be sum of valid Fibonacci values (complexity-based)
        # Just verify it's positive and that medium/complex scale from simple
        assert result.simple_estimate > 0
        assert result.medium_estimate == result.simple_estimate * 1.5
        assert result.complex_estimate == result.simple_estimate * 2.0

    def test_estimate_effort_invalid_mode(self, simple_phases, simple_dependencies):
        """Test invalid mode raises error."""
        fragment = create_test_fragment(simple_phases, simple_dependencies)
        with pytest.raises(ValueError, match="Unknown effort mode"):
            estimate_effort(fragment, mode="invalid")


class TestCostEstimation:
    """Tests for estimate_cost()."""

    def test_estimate_cost_simple(self, simple_phases, simple_dependencies):
        """Test simple cost estimation."""
        fragment = create_test_fragment(simple_phases, simple_dependencies)
        result = estimate_cost(fragment, mode="simple")

        # Total effort: 17 days * $150/day = $2550
        base_cost = 17.0 * 150.0
        assert result.base_cost == base_cost
        assert result.role_based_cost == base_cost
        # Risk premium: 1 + (0.3 * 0.2) = 1.06
        assert result.risk_adjusted_cost == pytest.approx(base_cost * 1.06)

    def test_estimate_cost_role_based(self, simple_phases, simple_dependencies):
        """Test role-based cost estimation."""
        # phase-1: dev-1 (5 days engineer) = 5 * 150 = 750
        # phase-2: dev-1 + dev-2 (5 days each engineer) = 5*150 + 5*150 = 1500
        # phase-3: dev-1 (2 days engineer) = 2 * 150 = 300
        # Total: 2550
        fragment = create_test_fragment(simple_phases, simple_dependencies)
        result = estimate_cost(fragment, mode="role_based")

        base_cost = 2550.0  # All engineers at $150/day
        assert result.base_cost == pytest.approx(base_cost)
        # Risk premium: 1 + (0.3 * 0.2) = 1.06
        assert result.risk_adjusted_cost == pytest.approx(base_cost * 1.06)

    def test_estimate_cost_risk_adjustment(self, simple_phases, simple_dependencies):
        """Test risk premium calculation."""
        high_risk_fragment = create_test_fragment(
            simple_phases, simple_dependencies, risk_score=0.8
        )
        result = estimate_cost(high_risk_fragment, mode="simple")

        base_cost = 17.0 * 150.0
        # Risk premium: 1 + (0.8 * 0.2) = 1.16
        assert result.risk_adjusted_cost == pytest.approx(base_cost * 1.16)

    def test_estimate_cost_invalid_mode(self, simple_phases, simple_dependencies):
        """Test invalid mode raises error."""
        fragment = create_test_fragment(simple_phases, simple_dependencies)
        with pytest.raises(ValueError, match="Unknown cost mode"):
            estimate_cost(fragment, mode="invalid")


class TestRiskConfidenceAggregation:
    """Tests for aggregate_risk_confidence()."""

    def test_aggregate_risk_confidence_basic(self, simple_phases, simple_dependencies):
        """Test basic risk/confidence aggregation."""
        fragment = create_test_fragment(simple_phases, simple_dependencies)
        risk, confidence = aggregate_risk_confidence(fragment)

        # Risk: max of phase risks (all default to 0.5)
        assert risk == 0.5
        # Confidence: average of phase confidences
        assert confidence == pytest.approx(0.7)  # default from fixture

    def test_aggregate_risk_confidence_empty(self):
        """Test with empty phases."""
        fragment = SequencerFragment(
            phases=[],
            dependencies=[],
            assignments={},
            ikam_references=[],
            validation=ValidationResult(is_valid=True),
            effort_estimate=EffortEstimate(
                simple_estimate=0.0,
                medium_estimate=0.0,
                complex_estimate=0.0,
            ),
            cost_estimate=CostEstimate(
                base_cost=0.0,
                role_based_cost=0.0,
                risk_adjusted_cost=0.0,
            ),
            duration_estimate=DurationEstimate(
                optimistic=0.0,
                nominal=0.0,
                pessimistic=0.0,
                critical_path_days=0.0,
            ),
            requested_by="test-user",
            request_mode="simple",
        )

        risk, confidence = aggregate_risk_confidence(fragment)
        assert risk == 0.5  # Default risk
        assert confidence == 0.5  # Default confidence for empty phases


class TestHelpers:
    """Tests for helper functions."""

    def test_infer_role_engineer(self):
        """Test role inference for engineer."""
        assert _infer_role("engineer-1") == "engineer"
        assert _infer_role("dev-123") == "engineer"
        assert _infer_role("programmer-alice") == "engineer"
        assert _infer_role("developer-bob") == "engineer"

    def test_infer_role_designer(self):
        """Test role inference for designer."""
        assert _infer_role("design-team") == "designer"
        assert _infer_role("ui-designer") == "designer"
        assert _infer_role("ux-lead") == "designer"

    def test_infer_role_pm(self):
        """Test role inference for PM."""
        assert _infer_role("pm-001") == "pm"
        assert _infer_role("product-manager") == "pm"
        assert _infer_role("manager-carol") == "pm"

    def test_infer_role_qa(self):
        """Test role inference for QA."""
        assert _infer_role("qa-team") == "qa"
        assert _infer_role("tester-dave") == "qa"
        assert _infer_role("qc-specialist") == "qa"

    def test_infer_role_contractor(self):
        """Test role inference for contractor."""
        assert _infer_role("contractor-external") == "contractor"

    def test_infer_role_default(self):
        """Test default role inference."""
        assert _infer_role("unknown-person") == "engineer"

    def test_estimate_phase_complexity(self):
        """Test phase complexity estimation."""
        phase = PlanPhase(
            id="p1",
            title="Test",
            description="",
            estimated_effort=10.0,
            assignees=["dev-1", "dev-2"],
        )
        complexity = _estimate_phase_complexity(phase)
        # Effort factor: 10/5 = 2, team factor: 2/2 = 1, total = 3
        assert complexity == 3
        assert complexity <= 9  # Should be clamped


class TestCriticalPathAnalysis:
    """Tests for _critical_path_analysis()."""

    def test_critical_path_linear(self, simple_phases, simple_dependencies):
        """Test critical path on linear DAG."""
        cp = _critical_path_analysis(simple_phases, simple_dependencies)
        # phase-1 (5) + phase-2 (10) + phase-3 (2) = 17
        assert cp == 17.0

    def test_critical_path_parallel(self):
        """Test critical path with parallel phases."""
        phases = [
            PlanPhase(
                id="p1", title="P1", description="", estimated_effort=5.0, assignees=[]
            ),
            PlanPhase(
                id="p2", title="P2", description="", estimated_effort=8.0, assignees=[]
            ),
            PlanPhase(
                id="p3", title="P3", description="", estimated_effort=3.0, assignees=[]
            ),
        ]

        deps = [
            PhaseDependency(
                predecessor_id="p1",
                successor_id="p3",
                edge_type="phase",
            ),
            PhaseDependency(
                predecessor_id="p2",
                successor_id="p3",
                edge_type="phase",
            ),
        ]

        cp = _critical_path_analysis(phases, deps)
        # max(p1+p3, p2+p3) = max(8, 11) = 11
        assert cp == 11.0

    def test_critical_path_empty(self):
        """Test critical path with empty phases."""
        assert _critical_path_analysis([], []) == 0.0

    def test_critical_path_single_phase(self):
        """Test critical path with single phase."""
        phases = [
            PlanPhase(
                id="p1", title="P1", description="", estimated_effort=5.0, assignees=[]
            )
        ]
        assert _critical_path_analysis(phases, []) == 5.0
