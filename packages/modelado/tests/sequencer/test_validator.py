"""Unit tests for sequencer DAG validator."""

import pytest
from unittest.mock import MagicMock

from modelado.sequencer.models import (
    SequencerFragment,
    PlanPhase,
    PhaseDependency,
    IKAMFragmentReference,
    ValidationResult,
    EffortEstimate,
    CostEstimate,
    DurationEstimate,
)
from modelado.sequencer.validator import (
    validate_sequence,
    _topological_sort,
    ValidationErrorCode,
    DAGValidationResult,
)


@pytest.fixture
def mock_connection():
    """Mock database connection for IKAM validation."""
    return MagicMock()


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
            assignees=["dev-1", "dev-2"],
        ),
    ]


def create_test_fragment(
    phases, dependencies, ikam_references=None
):
    """Helper to create a valid SequencerFragment for testing."""
    return SequencerFragment(
        phases=phases,
        dependencies=dependencies,
        assignments={phase.id: phase.assignees for phase in phases},
        ikam_references=ikam_references or [],
        validation=ValidationResult(
            is_valid=True,
            errors=[],
            warnings=[],
        ),
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
        risk_score=0.3,
        confidence_score=0.8,
        requested_by="test-user",
        request_mode="medium",
    )


class TestValidateSequence:
    """Tests for validate_sequence() function."""

    def test_validate_sequence_valid_phase_edges(self, simple_phases, mock_connection):
        """Test DAG validation passes for valid linear phase chain."""
        dependencies = [
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

        fragment = create_test_fragment(simple_phases, dependencies)
        result = validate_sequence(fragment, mock_connection)

        assert result.is_valid is True
        assert len(result.errors) == 0
        assert len(result.warnings) == 0
        assert result.phase_edge_count == 2
        assert result.artifact_edge_count == 0
        assert result.fragment_edge_count == 0
        assert result.topological_order == ["phase-1", "phase-2", "phase-3"]

    def test_validate_sequence_cycle_detection(self, simple_phases, mock_connection):
        """Test DAG validation detects cycles in phase dependencies."""
        dependencies = [
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
            PhaseDependency(
                predecessor_id="phase-3",
                successor_id="phase-1",
                dependency_type="finish_to_start",
                edge_type="phase",
            ),
        ]

        fragment = create_test_fragment(simple_phases, dependencies)
        result = validate_sequence(fragment, mock_connection)

        assert result.is_valid is False
        assert len(result.errors) > 0
        assert result.topological_order is None
        # Check for DAG_CYCLE error
        cycle_errors = [e for e in result.errors if e.code == ValidationErrorCode.DAG_CYCLE]
        assert len(cycle_errors) > 0

    def test_validate_sequence_missing_predecessor(self, simple_phases, mock_connection):
        """Test DAG validation detects missing phase predecessors."""
        dependencies = [
            PhaseDependency(
                predecessor_id="phase-nonexistent",
                successor_id="phase-1",
                dependency_type="finish_to_start",
                edge_type="phase",
            ),
        ]

        fragment = create_test_fragment(simple_phases, dependencies)
        result = validate_sequence(fragment, mock_connection)

        assert result.is_valid is False
        missing_pred_errors = [
            e for e in result.errors
            if e.code == ValidationErrorCode.MISSING_PREDECESSOR
        ]
        assert len(missing_pred_errors) > 0

    def test_validate_sequence_mixed_edge_types(self, simple_phases, mock_connection):
        """Test DAG validation with mixed phase/artifact/fragment edges."""
        dependencies = [
            PhaseDependency(
                predecessor_id="phase-1",
                successor_id="phase-2",
                dependency_type="finish_to_start",
                edge_type="phase",
            ),
            PhaseDependency(
                predecessor_id="artifact-123",
                successor_id="phase-2",
                dependency_type="required_input",
                edge_type="artifact",
            ),
            PhaseDependency(
                predecessor_id="fragment-456",
                successor_id="phase-3",
                dependency_type="required_input",
                edge_type="fragment",
            ),
        ]

        ikam_references = [
            IKAMFragmentReference(
                artifact_id="artifact-123",
                fragment_id=None,
                reference_type="input_from_data",
                scope=["phase-2"],
                metadata={},
            ),
            IKAMFragmentReference(
                artifact_id="artifact-789",
                fragment_id="fragment-456",
                reference_type="depends_on_formula",
                scope=["phase-3"],
                metadata={},
            ),
        ]

        fragment = create_test_fragment(simple_phases, dependencies, ikam_references)
        result = validate_sequence(fragment, mock_connection)

        assert result.phase_edge_count == 1
        assert result.artifact_edge_count == 1
        assert result.fragment_edge_count == 1

    def test_validate_sequence_missing_assignees(self, simple_phases, mock_connection):
        """Test DAG validation detects phases with missing or empty assignees."""
        # Create a phase with no assignees
        phases_with_missing = simple_phases.copy()
        phases_with_missing[1].assignees = []

        dependencies = [
            PhaseDependency(
                predecessor_id="phase-1",
                successor_id="phase-2",
                dependency_type="finish_to_start",
                edge_type="phase",
            ),
        ]

        fragment = create_test_fragment(phases_with_missing, dependencies)
        result = validate_sequence(fragment, mock_connection)

        assert result.is_valid is False
        empty_assignee_errors = [
            e for e in result.errors
            if e.code == ValidationErrorCode.EMPTY_ASSIGNEE_LIST
        ]
        assert len(empty_assignee_errors) > 0

    def test_validate_sequence_invalid_edge_type(self, simple_phases, mock_connection):
        """Test DAG validation counts and validates edge types correctly."""
        # Test with all three valid edge types
        dependencies = [
            PhaseDependency(
                predecessor_id="phase-1",
                successor_id="phase-2",
                dependency_type="finish_to_start",
                edge_type="phase",
            ),
            PhaseDependency(
                predecessor_id="artifact-123",
                successor_id="phase-2",
                dependency_type="required_input",
                edge_type="artifact",
            ),
            PhaseDependency(
                predecessor_id="fragment-456",
                successor_id="phase-3",
                dependency_type="required_input",
                edge_type="fragment",
            ),
        ]

        fragment = create_test_fragment(simple_phases, dependencies)
        result = validate_sequence(fragment, mock_connection)

        # All valid edge types should pass validation
        assert result.is_valid is True
        assert result.phase_edge_count == 1
        assert result.artifact_edge_count == 1
        assert result.fragment_edge_count == 1
        # No invalid edge type errors should be present
        invalid_edge_errors = [
            e for e in result.errors
            if e.code == ValidationErrorCode.INVALID_EDGE_TYPE
        ]
        assert len(invalid_edge_errors) == 0

    def test_validate_sequence_orphaned_phase_warning(self, simple_phases, mock_connection):
        """Test DAG validation warns about orphaned phases (no dependencies)."""
        dependencies = [
            PhaseDependency(
                predecessor_id="phase-1",
                successor_id="phase-2",
                dependency_type="finish_to_start",
                edge_type="phase",
            ),
        ]
        # phase-3 has no dependencies (orphaned)

        fragment = create_test_fragment(simple_phases, dependencies)
        result = validate_sequence(fragment, mock_connection)

        # Valid, but with warnings
        assert result.is_valid is True
        orphaned_warnings = [
            w for w in result.warnings
            if w.code == ValidationErrorCode.ORPHANED_PHASE
        ]
        assert len(orphaned_warnings) > 0


class TestTopologicalSort:
    """Tests for _topological_sort() helper function."""

    def test_topological_sort_linear(self, simple_phases):
        """Test topological sort on linear phase chain."""
        dependencies = [
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

        result = _topological_sort(simple_phases, dependencies)

        assert result is not None
        assert result == ["phase-1", "phase-2", "phase-3"]

    def test_topological_sort_parallel(self):
        """Test topological sort with parallel phases."""
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
                description="Second phase",
                estimated_effort=10.0,
                assignees=["dev-1"],
            ),
            PlanPhase(
                id="phase-3",
                title="Phase 3",
                description="Third phase",
                estimated_effort=2.0,
                assignees=["dev-1"],
            ),
            PlanPhase(
                id="phase-4",
                title="Phase 4",
                description="Fourth phase",
                estimated_effort=8.0,
                assignees=["dev-1"],
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
                predecessor_id="phase-1",
                successor_id="phase-2",
                dependency_type="finish_to_start",
                edge_type="phase",
            ),
            PhaseDependency(
                predecessor_id="phase-2",
                successor_id="phase-4",
                dependency_type="finish_to_start",
                edge_type="phase",
            ),
            PhaseDependency(
                predecessor_id="phase-3",
                successor_id="phase-4",
                dependency_type="finish_to_start",
                edge_type="phase",
            ),
        ]

        result = _topological_sort(phases, dependencies)

        assert result is not None
        # phase-1 must come before phase-2 and phase-3
        assert result.index("phase-1") < result.index("phase-2")
        assert result.index("phase-1") < result.index("phase-3")
        # phase-2 and phase-3 must come before phase-4
        assert result.index("phase-2") < result.index("phase-4")
        assert result.index("phase-3") < result.index("phase-4")

    def test_topological_sort_cycle(self, simple_phases):
        """Test topological sort returns None when cycle detected."""
        dependencies = [
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
            PhaseDependency(
                predecessor_id="phase-3",
                successor_id="phase-1",
                dependency_type="finish_to_start",
                edge_type="phase",
            ),
        ]

        result = _topological_sort(simple_phases, dependencies)

        assert result is None
