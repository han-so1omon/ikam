"""
Unit tests for sequencer models and adapters (Phase 6, Issue #24).

Tests cover:
- Model creation and validation
- JSON serialization/deserialization
- Domain/storage adapter round-trips
- IKAM reference field behavior
- Edge cases and error handling

All tests validate that Phase 6 data models support:
1. Lossless round-trip conversion (storage → domain → storage = original)
2. JSON schema compliance (Pydantic validation)
3. IKAM reference integration (artifact_id, fragment_id, scope tracking)
"""

import pytest
import json
from datetime import datetime, UTC
from uuid import uuid4

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

from modelado.sequencer.adapters import (
    sequencer_domain_to_storage,
    sequencer_storage_to_domain,
    project_phase_domain_to_storage,
    project_phase_storage_to_domain,
    SerializationError,
    DeserializationError,
)


# ============================================================================
# Test Fixtures
# ============================================================================

@pytest.fixture
def sample_ikam_reference():
    """Sample IKAM fragment reference."""
    return IKAMFragmentReference(
        artifact_id=str(uuid4()),
        fragment_id="frag-cost-model-001",
        reference_type="uses_variable",
        scope=["phase-1", "phase-2"],
        metadata={
            "artifact_kind": "EconomicModel",
            "artifact_title": "SaaS Unit Economics v3",
            "confidence": 0.9
        }
    )


@pytest.fixture
def sample_plan_phase():
    """Sample plan phase with IKAM inputs/outputs."""
    return PlanPhase(
        id="phase-1",
        title="Economic Modeling",
        description="Build unit economics model",
        estimated_effort=5.0,
        assignees=["analyst-001"],
        ikam_inputs=["artifact-data-001"],
        ikam_outputs=["artifact-model-001"],
        risk_score=0.3
    )


@pytest.fixture
def sample_phase_dependency():
    """Sample phase dependency (phase edge type)."""
    return PhaseDependency(
        predecessor_id="phase-1",
        successor_id="phase-2",
        dependency_type="finish_to_start",
        edge_type="phase"
    )


@pytest.fixture
def sample_validation_result():
    """Sample validation result with errors and warnings."""
    return ValidationResult(
        is_valid=False,
        errors=[
            ValidationError(
                code="DAG_CYCLE",
                message="Circular dependency detected: phase-1 → phase-2 → phase-1",
                severity="ERROR",
                phase_ids=["phase-1", "phase-2"]
            )
        ],
        warnings=[
            ValidationError(
                code="MISSING_IKAM_REFERENCE",
                message="Referenced artifact not found in IKAM",
                severity="WARNING"
            )
        ]
    )


@pytest.fixture
def sample_sequencer_fragment(sample_plan_phase, sample_phase_dependency, sample_ikam_reference):
    """Complete SequencerFragment for testing."""
    return SequencerFragment(
        phases=[sample_plan_phase],
        dependencies=[sample_phase_dependency],
        assignments={"phase-1": ["analyst-001"]},
        ikam_references=[sample_ikam_reference],
        validation=ValidationResult(is_valid=True, errors=[], warnings=[]),
        effort_estimate=EffortEstimate(
            simple_estimate=5.0,
            medium_estimate=6.5,
            complex_estimate=8.0,
            unit="person-days"
        ),
        cost_estimate=CostEstimate(
            base_cost=5000.0,
            role_based_cost=6500.0,
            risk_adjusted_cost=7800.0,
            currency="USD"
        ),
        duration_estimate=DurationEstimate(
            optimistic=3.0,
            nominal=5.0,
            pessimistic=8.0,
            critical_path_days=5.0
        ),
        risk_score=0.3,
        confidence_score=0.85,
        derived_from_instruction_id="instr-123",
        requested_by="user-456",
        request_mode="medium",
        created_at=datetime.now(UTC)
    )


# ============================================================================
# Model Creation Tests
# ============================================================================

def test_ikam_reference_creation(sample_ikam_reference):
    """Test IKAMFragmentReference creation and field access."""
    assert sample_ikam_reference.artifact_id is not None
    assert sample_ikam_reference.fragment_id == "frag-cost-model-001"
    assert sample_ikam_reference.reference_type == "uses_variable"
    assert len(sample_ikam_reference.scope) == 2
    assert sample_ikam_reference.metadata["confidence"] == 0.9


def test_plan_phase_creation_with_ikam_fields(sample_plan_phase):
    """Test PlanPhase creation with ikam_inputs and ikam_outputs fields."""
    assert sample_plan_phase.id == "phase-1"
    assert len(sample_plan_phase.ikam_inputs) == 1
    assert len(sample_plan_phase.ikam_outputs) == 1
    assert sample_plan_phase.risk_score == 0.3


def test_phase_dependency_edge_types():
    """Test PhaseDependency with all 3 edge types."""
    # Phase edge
    phase_dep = PhaseDependency(
        predecessor_id="phase-1",
        successor_id="phase-2",
        edge_type="phase"
    )
    assert phase_dep.edge_type == "phase"
    
    # Artifact edge
    artifact_dep = PhaseDependency(
        predecessor_id=str(uuid4()),
        successor_id="phase-3",
        edge_type="artifact"
    )
    assert artifact_dep.edge_type == "artifact"
    
    # Fragment edge
    fragment_dep = PhaseDependency(
        predecessor_id="frag-001",
        successor_id="phase-4",
        edge_type="fragment"
    )
    assert fragment_dep.edge_type == "fragment"


def test_sequencer_fragment_with_ikam_references(sample_sequencer_fragment):
    """Test SequencerFragment includes ikam_references field."""
    assert len(sample_sequencer_fragment.ikam_references) == 1
    assert sample_sequencer_fragment.ikam_references[0].reference_type == "uses_variable"
    assert sample_sequencer_fragment.confidence_score == 0.85


# ============================================================================
# JSON Serialization Tests
# ============================================================================

def test_sequencer_fragment_json_serialization(sample_sequencer_fragment):
    """Test SequencerFragment serializes to JSON and deserializes losslessly."""
    # Serialize
    json_dict = sample_sequencer_fragment.model_dump(mode='json')
    json_str = json.dumps(json_dict)
    
    # Deserialize
    loaded_dict = json.loads(json_str)
    reconstructed = SequencerFragment(**loaded_dict)
    
    # Validate
    assert reconstructed.phases[0].id == sample_sequencer_fragment.phases[0].id
    assert len(reconstructed.ikam_references) == 1
    assert reconstructed.confidence_score == sample_sequencer_fragment.confidence_score


def test_ikam_reference_json_serialization(sample_ikam_reference):
    """Test IKAMFragmentReference serializes to JSON."""
    json_dict = sample_ikam_reference.model_dump(mode='json')
    assert json_dict["reference_type"] == "uses_variable"
    assert len(json_dict["scope"]) == 2


def test_project_phase_fragment_json_serialization():
    """Test ProjectPhaseFragment serializes to JSON."""
    committed = CommittedPhase(
        id="phase-1",
        title="Test Phase",
        description="Test",
        estimated_effort=5.0,
        assignees=["user-1"],
        ikam_inputs=["input-1"],
        ikam_outputs=["output-1"],
        status="planned"
    )
    
    fragment = ProjectPhaseFragment(
        phases=[committed],
        dependencies=[],
        assignments={"phase-1": ["user-1"]},
        derived_from="seq-frag-123",
        sequencer_request_id="req-456",
        committed_by="user-789",
        committed_at=datetime.now(UTC)
    )
    
    json_dict = fragment.model_dump(mode='json')
    assert json_dict["phases"][0]["status"] == "planned"
    assert json_dict["derivation_type"] == "sequencer_fragment_committed"


# ============================================================================
# Adapter Round-Trip Tests
# ============================================================================

def test_sequencer_adapter_round_trip(sample_sequencer_fragment):
    """Test domain → storage → domain round-trip is lossless."""
    # Domain → Storage
    storage_fragment, metadata = sequencer_domain_to_storage(sample_sequencer_fragment)
    
    # Validate storage
    assert storage_fragment.id is not None  # Blake3 hash
    assert storage_fragment.mime_type == "application/vnd.ikam+sequencer+json"
    assert storage_fragment.size > 0
    assert metadata["type"] == "structural"
    
    # Storage → Domain
    reconstructed = sequencer_storage_to_domain(storage_fragment, metadata)
    
    # Validate round-trip
    assert reconstructed.phases[0].id == sample_sequencer_fragment.phases[0].id
    assert len(reconstructed.ikam_references) == len(sample_sequencer_fragment.ikam_references)
    assert reconstructed.confidence_score == sample_sequencer_fragment.confidence_score
    assert reconstructed.request_mode == sample_sequencer_fragment.request_mode


def test_project_phase_adapter_round_trip():
    """Test ProjectPhaseFragment domain → storage → domain round-trip."""
    # Create domain fragment
    committed = CommittedPhase(
        id="phase-1",
        title="Test Phase",
        description="Test description",
        estimated_effort=5.0,
        assignees=["user-1"],
        ikam_inputs=["input-1"],
        ikam_outputs=["output-1"],
        status="in_progress"
    )
    
    original = ProjectPhaseFragment(
        phases=[committed],
        dependencies=[],
        assignments={"phase-1": ["user-1"]},
        derived_from="seq-frag-123",
        sequencer_request_id="req-456",
        committed_by="user-789",
        committed_at=datetime.now(UTC)
    )
    
    # Domain → Storage
    storage_fragment, metadata = project_phase_domain_to_storage(original)
    
    # Storage → Domain
    reconstructed = project_phase_storage_to_domain(storage_fragment, metadata)
    
    # Validate round-trip
    assert reconstructed.phases[0].id == original.phases[0].id
    assert reconstructed.phases[0].status == original.phases[0].status
    assert reconstructed.derived_from == original.derived_from


def test_adapter_deterministic_cas_id(sample_sequencer_fragment):
    """Test that same content produces same Blake3 hash."""
    storage1, _ = sequencer_domain_to_storage(sample_sequencer_fragment)
    storage2, _ = sequencer_domain_to_storage(sample_sequencer_fragment)
    
    assert storage1.id == storage2.id  # Deterministic hash


# ============================================================================
# IKAM Reference Field Tests
# ============================================================================

def test_ikam_reference_with_all_reference_types():
    """Test all 5 reference_type values are valid."""
    reference_types = [
        "uses_variable",
        "depends_on_formula",
        "input_from_data",
        "output_to_narrative",
        "extends_model"
    ]
    
    for ref_type in reference_types:
        ref = IKAMFragmentReference(
            artifact_id=str(uuid4()),
            reference_type=ref_type,
            scope=["phase-1"]
        )
        assert ref.reference_type == ref_type


def test_ikam_reference_optional_fragment_id():
    """Test fragment_id is optional (can reference entire artifact)."""
    # Without fragment_id (references entire artifact)
    ref1 = IKAMFragmentReference(
        artifact_id=str(uuid4()),
        reference_type="uses_variable",
        scope=[]
    )
    assert ref1.fragment_id is None
    
    # With fragment_id (references specific fragment)
    ref2 = IKAMFragmentReference(
        artifact_id=str(uuid4()),
        fragment_id="frag-123",
        reference_type="uses_variable",
        scope=[]
    )
    assert ref2.fragment_id == "frag-123"


def test_sequencer_fragment_empty_ikam_references():
    """Test SequencerFragment with no IKAM references (valid scenario)."""
    fragment = SequencerFragment(
        phases=[
            PlanPhase(
                id="phase-1",
                title="Simple Phase",
                estimated_effort=3.0
            )
        ],
        ikam_references=[],  # No references
        validation=ValidationResult(is_valid=True),
        effort_estimate=EffortEstimate(
            simple_estimate=3.0,
            medium_estimate=3.5,
            complex_estimate=4.0
        ),
        cost_estimate=CostEstimate(
            base_cost=3000.0,
            role_based_cost=3500.0,
            risk_adjusted_cost=4000.0
        ),
        duration_estimate=DurationEstimate(
            optimistic=2.0,
            nominal=3.0,
            pessimistic=5.0,
            critical_path_days=3.0
        ),
        requested_by="user-123",
        request_mode="simple"
    )
    
    assert len(fragment.ikam_references) == 0
    assert fragment.validation.is_valid


# ============================================================================
# Validation Tests
# ============================================================================

def test_validation_result_with_errors_and_warnings(sample_validation_result):
    """Test ValidationResult with both errors and warnings."""
    assert not sample_validation_result.is_valid
    assert len(sample_validation_result.errors) == 1
    assert len(sample_validation_result.warnings) == 1
    assert sample_validation_result.errors[0].code == "DAG_CYCLE"


def test_pydantic_validation_catches_invalid_effort():
    """Test Pydantic validation catches negative effort."""
    with pytest.raises(Exception):  # Pydantic ValidationError
        PlanPhase(
            id="phase-1",
            title="Bad Phase",
            estimated_effort=-5.0  # Invalid: negative effort
        )


def test_pydantic_validation_catches_invalid_risk_score():
    """Test Pydantic validation catches risk_score outside [0, 1] range."""
    with pytest.raises(Exception):  # Pydantic ValidationError
        PlanPhase(
            id="phase-1",
            title="Bad Phase",
            estimated_effort=5.0,
            risk_score=1.5  # Invalid: > 1.0
        )


# ============================================================================
# Edge Case Tests
# ============================================================================

def test_sequencer_fragment_with_multiple_ikam_references():
    """Test SequencerFragment with multiple IKAM references."""
    refs = [
        IKAMFragmentReference(
            artifact_id=str(uuid4()),
            reference_type="uses_variable",
            scope=["phase-1"]
        ),
        IKAMFragmentReference(
            artifact_id=str(uuid4()),
            reference_type="input_from_data",
            scope=["phase-1", "phase-2"]
        ),
        IKAMFragmentReference(
            artifact_id=str(uuid4()),
            fragment_id="frag-narrative-001",
            reference_type="output_to_narrative",
            scope=["phase-3"]
        )
    ]
    
    fragment = SequencerFragment(
        phases=[
            PlanPhase(id="phase-1", title="P1", estimated_effort=3.0),
            PlanPhase(id="phase-2", title="P2", estimated_effort=2.0),
            PlanPhase(id="phase-3", title="P3", estimated_effort=4.0)
        ],
        ikam_references=refs,
        validation=ValidationResult(is_valid=True),
        effort_estimate=EffortEstimate(
            simple_estimate=9.0,
            medium_estimate=10.0,
            complex_estimate=12.0
        ),
        cost_estimate=CostEstimate(
            base_cost=9000.0,
            role_based_cost=10000.0,
            risk_adjusted_cost=12000.0
        ),
        duration_estimate=DurationEstimate(
            optimistic=7.0,
            nominal=9.0,
            pessimistic=12.0,
            critical_path_days=9.0
        ),
        requested_by="user-123",
        request_mode="medium"
    )
    
    assert len(fragment.ikam_references) == 3
    assert fragment.ikam_references[2].fragment_id == "frag-narrative-001"


def test_phase_dependency_artifact_edge_type():
    """Test PhaseDependency with artifact edge_type (phase depends on IKAM artifact)."""
    dep = PhaseDependency(
        predecessor_id=str(uuid4()),  # artifact UUID
        successor_id="phase-2",
        dependency_type="finish_to_start",
        edge_type="artifact"
    )
    
    assert dep.edge_type == "artifact"
    # Serialize and deserialize
    json_dict = dep.model_dump(mode='json')
    reconstructed = PhaseDependency(**json_dict)
    assert reconstructed.edge_type == "artifact"
