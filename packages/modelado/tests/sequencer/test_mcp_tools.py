"""Tests for MCP sequencer tools."""

import pytest
from unittest.mock import Mock, MagicMock
from datetime import datetime
from modelado.sequencer.mcp_tools import (
    create_sequence,
    validate_sequence_tool,
    commit_sequence,
    MCP_TOOLS,
)
from modelado.sequencer.models import SequencerFragment


@pytest.fixture
def sample_phases():
    """Sample phase data for testing."""
    return [
        {
            "id": "phase-1",
            "title": "Requirements Gathering",
            "description": "Gather and document requirements",
            "estimated_effort": 5.0,
            "assignees": ["analyst-1"],
            "risk_score": 0.3,
        },
        {
            "id": "phase-2",
            "title": "Design",
            "description": "Create system design",
            "estimated_effort": 10.0,
            "assignees": ["architect-1", "designer-1"],
            "risk_score": 0.5,
        },
        {
            "id": "phase-3",
            "title": "Implementation",
            "description": "Build the system",
            "estimated_effort": 20.0,
            "assignees": ["dev-1", "dev-2", "dev-3"],
            "risk_score": 0.6,
        },
    ]


@pytest.fixture
def sample_dependencies():
    """Sample dependencies for testing."""
    return [
        {
            "predecessor_id": "phase-1",
            "successor_id": "phase-2",
            "edge_type": "phase",
            "dependency_type": "finish_to_start",
        },
        {
            "predecessor_id": "phase-2",
            "successor_id": "phase-3",
            "edge_type": "phase",
            "dependency_type": "finish_to_start",
        },
    ]


@pytest.fixture
def sample_ikam_references():
    """Sample IKAM references for testing."""
    return [
        {
            "artifact_id": "cost-model-123",
            "reference_type": "uses_variable",
            "scope": ["phase-2", "phase-3"],
            "metadata": {"variable_name": "unit_cost", "confidence": 0.9},
        },
    ]


class TestCreateSequence:
    """Test create_sequence MCP tool."""

    def test_create_sequence_simple_mode(self, sample_phases, sample_dependencies):
        """Test creating sequence with simple estimation mode."""
        result = create_sequence(
            instruction="Build a new feature with requirements, design, and implementation phases",
            phases=sample_phases,
            dependencies=sample_dependencies,
            requested_by="test-user",
            request_mode="simple",
        )

        assert result["status"] == "success"
        assert "sequencer_fragment" in result
        assert "validation" in result
        assert "estimates" in result
        assert result["validation"]["is_valid"] is True
        assert result["estimates"]["duration"]["nominal"] > 0
        assert result["estimates"]["effort"]["simple"] > 0
        assert result["estimates"]["cost"]["base"] > 0

    def test_create_sequence_medium_mode(self, sample_phases, sample_dependencies):
        """Test creating sequence with medium estimation mode."""
        result = create_sequence(
            instruction="Build feature with critical path analysis",
            phases=sample_phases,
            dependencies=sample_dependencies,
            requested_by="test-user",
            request_mode="medium",
        )

        assert result["status"] == "success"
        assert result["estimates"]["duration"]["critical_path_days"] > 0
        assert result["estimates"]["cost"]["role_based"] > 0

    def test_create_sequence_complex_mode(self, sample_phases, sample_dependencies):
        """Test creating sequence with complex estimation mode (Fibonacci effort)."""
        result = create_sequence(
            instruction="Build feature with Fibonacci effort estimation",
            phases=sample_phases,
            dependencies=sample_dependencies,
            requested_by="test-user",
            request_mode="complex",
        )

        assert result["status"] == "success"
        assert result["estimates"]["effort"]["complex"] > 0
        assert result["estimates"]["cost"]["risk_adjusted"] > 0

    def test_create_sequence_with_ikam_references(
        self, sample_phases, sample_dependencies, sample_ikam_references
    ):
        """Test creating sequence with IKAM references."""
        result = create_sequence(
            instruction="Build feature referencing cost model",
            phases=sample_phases,
            dependencies=sample_dependencies,
            ikam_references=sample_ikam_references,
            requested_by="test-user",
            request_mode="medium",
        )

        assert result["status"] in ["success", "validation_failed"]  # IKAM resolution may fail in tests
        fragment = result["sequencer_fragment"]
        assert len(fragment["ikam_references"]) == 1

    def test_create_sequence_with_instruction_id(self, sample_phases, sample_dependencies):
        """Test creating sequence with instruction ID for provenance."""
        result = create_sequence(
            instruction="Build feature",
            phases=sample_phases,
            dependencies=sample_dependencies,
            requested_by="test-user",
            request_mode="simple",
            instruction_id="instruction-abc-123",
        )

        assert result["instruction_id"] == "instruction-abc-123"
        fragment = result["sequencer_fragment"]
        assert fragment["derived_from_instruction_id"] == "instruction-abc-123"

    def test_create_sequence_invalid_dag(self, sample_phases):
        """Test creating sequence with cycle in DAG."""
        cyclic_deps = [
            {"predecessor_id": "phase-1", "successor_id": "phase-2", "edge_type": "phase"},
            {"predecessor_id": "phase-2", "successor_id": "phase-3", "edge_type": "phase"},
            {"predecessor_id": "phase-3", "successor_id": "phase-1", "edge_type": "phase"},  # Cycle!
        ]

        result = create_sequence(
            instruction="Build feature with circular dependencies",
            phases=sample_phases,
            dependencies=cyclic_deps,
            requested_by="test-user",
            request_mode="simple",
        )

        assert result["status"] == "validation_failed"
        assert not result["validation"]["is_valid"]
        assert len(result["validation"]["errors"]) > 0

    def test_create_sequence_missing_predecessor(self, sample_phases):
        """Test creating sequence with missing predecessor phase."""
        invalid_deps = [
            {"predecessor_id": "nonexistent-phase", "successor_id": "phase-2", "edge_type": "phase"},
        ]

        result = create_sequence(
            instruction="Build feature",
            phases=sample_phases,
            dependencies=invalid_deps,
            requested_by="test-user",
            request_mode="simple",
        )

        assert result["status"] == "validation_failed"
        assert not result["validation"]["is_valid"]

    def test_create_sequence_estimates_accuracy(self, sample_phases, sample_dependencies):
        """Test that estimates are computed correctly."""
        result = create_sequence(
            instruction="Build feature",
            phases=sample_phases,
            dependencies=sample_dependencies,
            requested_by="test-user",
            request_mode="simple",
        )

        # Total effort = 5 + 10 + 20 = 35 person-days
        assert result["estimates"]["effort"]["simple"] == 35.0
        
        # Duration (simple mode) = sum of efforts = 35 days
        assert result["estimates"]["duration"]["nominal"] == 35.0

        # Risk score should be max(0.3, 0.5, 0.6) = 0.6
        assert result["estimates"]["risk_score"] == 0.6


class TestValidateSequenceTool:
    """Test validate_sequence_tool MCP tool."""

    def test_validate_valid_sequence(self, sample_phases, sample_dependencies):
        """Test validating a valid sequence."""
        # First create a sequence
        create_result = create_sequence(
            instruction="Build feature",
            phases=sample_phases,
            dependencies=sample_dependencies,
            requested_by="test-user",
            request_mode="simple",
        )

        # Then validate it
        validate_result = validate_sequence_tool(
            sequencer_fragment_dict=create_result["sequencer_fragment"]
        )

        assert validate_result["status"] == "valid"
        assert validate_result["validation"]["is_valid"] is True
        assert len(validate_result["validation"]["errors"]) == 0

    def test_validate_invalid_sequence(self, sample_phases):
        """Test validating an invalid sequence with cycle."""
        cyclic_deps = [
            {"predecessor_id": "phase-1", "successor_id": "phase-2", "edge_type": "phase"},
            {"predecessor_id": "phase-2", "successor_id": "phase-3", "edge_type": "phase"},
            {"predecessor_id": "phase-3", "successor_id": "phase-1", "edge_type": "phase"},
        ]

        create_result = create_sequence(
            instruction="Build feature",
            phases=sample_phases,
            dependencies=cyclic_deps,
            requested_by="test-user",
            request_mode="simple",
        )

        validate_result = validate_sequence_tool(
            sequencer_fragment_dict=create_result["sequencer_fragment"]
        )

        assert validate_result["status"] == "invalid"
        assert not validate_result["validation"]["is_valid"]
        assert len(validate_result["validation"]["errors"]) > 0


class TestCommitSequence:
    """Test commit_sequence MCP tool."""

    def test_commit_valid_sequence(self, sample_phases, sample_dependencies):
        """Test committing a valid sequence to execution."""
        # Create sequence
        create_result = create_sequence(
            instruction="Build feature",
            phases=sample_phases,
            dependencies=sample_dependencies,
            requested_by="test-user",
            request_mode="simple",
        )

        # Add ID for provenance (normally from database)
        create_result["sequencer_fragment"]["id"] = "seq-fragment-123"

        # Commit sequence
        commit_result = commit_sequence(
            sequencer_fragment_dict=create_result["sequencer_fragment"],
            committed_by="project-manager",
            sequencer_request_id="request-abc-456",
        )

        assert commit_result["status"] == "committed"
        assert "project_phase_fragment" in commit_result
        assert "provenance" in commit_result
        assert "summary" in commit_result

        # Check provenance
        provenance = commit_result["provenance"]
        assert provenance["derived_from"] == "seq-fragment-123"
        assert provenance["derivation_type"] == "sequencer_fragment_committed"
        assert provenance["sequencer_request_id"] == "request-abc-456"
        assert provenance["committed_by"] == "project-manager"

        # Check summary
        summary = commit_result["summary"]
        assert summary["total_phases"] == 3
        assert summary["total_effort"] == 35.0

    def test_commit_invalid_sequence(self, sample_phases):
        """Test that committing invalid sequence returns error."""
        # Create invalid sequence
        cyclic_deps = [
            {"predecessor_id": "phase-1", "successor_id": "phase-2", "edge_type": "phase"},
            {"predecessor_id": "phase-2", "successor_id": "phase-1", "edge_type": "phase"},
        ]

        create_result = create_sequence(
            instruction="Build feature",
            phases=sample_phases,
            dependencies=cyclic_deps,
            requested_by="test-user",
            request_mode="simple",
        )

        # Try to commit
        commit_result = commit_sequence(
            sequencer_fragment_dict=create_result["sequencer_fragment"],
            committed_by="project-manager",
            sequencer_request_id="request-abc-456",
        )

        assert commit_result["status"] == "error"
        assert "error" in commit_result
        assert "validation_errors" in commit_result

    def test_commit_sequence_creates_committed_phases(self, sample_phases, sample_dependencies):
        """Test that commit creates CommittedPhase objects."""
        create_result = create_sequence(
            instruction="Build feature",
            phases=sample_phases,
            dependencies=sample_dependencies,
            requested_by="test-user",
            request_mode="simple",
        )
        create_result["sequencer_fragment"]["id"] = "seq-123"

        commit_result = commit_sequence(
            sequencer_fragment_dict=create_result["sequencer_fragment"],
            committed_by="pm",
            sequencer_request_id="req-456",
        )

        project_fragment = commit_result["project_phase_fragment"]
        assert len(project_fragment["phases"]) == 3
        
        # All phases should have status "planned"
        for phase in project_fragment["phases"]:
            assert phase["status"] == "planned"

    def test_commit_sequence_with_ikam_references_no_db(self, sample_phases, sample_dependencies):
        """Test commit with IKAM references but no database connection (skipped resolution)."""
        # Create sequence with IKAM references
        ikam_refs = [
            {
                "artifact_id": "cost-model-v1",
                "fragment_id": None,
                "reference_type": "uses_variable",
                "scope": ["phase-1"],
                "metadata": {"variable_name": "unit_cost"},
            }
        ]
        
        create_result = create_sequence(
            instruction="Build with cost model reference",
            phases=sample_phases,
            dependencies=sample_dependencies,
            ikam_references=ikam_refs,
            requested_by="test-user",
            request_mode="medium",
        )
        
        assert create_result["status"] == "success"
        create_result["sequencer_fragment"]["id"] = "seq-ikam-001"
        
        # Commit without database connection
        commit_result = commit_sequence(
            sequencer_fragment_dict=create_result["sequencer_fragment"],
            committed_by="pm",
            sequencer_request_id="req-ikam-001",
            connection=None,  # Explicitly no connection
        )
        
        assert commit_result["status"] == "committed"
        assert commit_result["ikam_resolution"]["status"] == "skipped_no_connection"
        assert commit_result["ikam_resolution"]["references_processed"] == 1
        assert commit_result["ikam_resolution"]["derivation_events_emitted"] == 0

    def test_commit_sequence_with_mocked_db_ikam_resolution(self, sample_phases, sample_dependencies):
        """Test commit with mocked database IKAM resolution and derivation event emission."""
        # Create sequence with IKAM references
        ikam_refs = [
            {
                "artifact_id": "cost-model-v1",
                "fragment_id": None,
                "reference_type": "uses_variable",
                "scope": ["phase-1"],
                "metadata": {"variable_name": "unit_cost"},
            },
            {
                "artifact_id": "revenue-forecast-q4",
                "fragment_id": None,
                "reference_type": "depends_on_formula",
                "scope": ["phase-2"],
                "metadata": {"formula_name": "annual_revenue"},
            },
        ]
        
        create_result = create_sequence(
            instruction="Build with cost and revenue references",
            phases=sample_phases,
            dependencies=sample_dependencies,
            ikam_references=ikam_refs,
            requested_by="test-user",
            request_mode="complex",
        )
        
        assert create_result["status"] == "success"
        create_result["sequencer_fragment"]["id"] = "seq-ikam-db-001"
        
        # Mock database connection and resolve_ikam_references/validate_ikam_references
        mock_connection = Mock()
        mock_cursor = MagicMock()
        mock_connection.cursor.return_value.__enter__ = Mock(return_value=mock_cursor)
        mock_connection.cursor.return_value.__exit__ = Mock(return_value=False)
        
        # Mock artifact lookup responses
        def mock_execute(query, params=None):
            if "ikam_artifacts" in query:
                mock_cursor.fetchall.return_value = [
                    ("cost-model-v1", "EconomicModel", "SaaS Unit Economics Model"),
                    ("revenue-forecast-q4", "DataForecast", "Q4 Revenue Forecast"),
                ]
            elif "ikam_fragment_meta" in query:
                # Return mock fragments
                mock_cursor.fetchall.return_value = [
                    ("frag-001", 0),
                    ("frag-002", 1),
                ]
            return None
        
        mock_cursor.execute.side_effect = mock_execute
        
        # Patch resolve_ikam_references to return mocked data
        from modelado.sequencer import mcp_tools
        original_resolve = mcp_tools.resolve_ikam_references
        
        def mock_resolve(artifact_ids, connection):
            return {
                "cost-model-v1": {
                    "status": "RESOLVED",
                    "artifact_id": "cost-model-v1",
                    "artifact_kind": "EconomicModel",
                    "artifact_title": "SaaS Unit Economics Model",
                    "fragments": [{"id": "frag-001", "level": 0}, {"id": "frag-002", "level": 1}],
                },
                "revenue-forecast-q4": {
                    "status": "RESOLVED",
                    "artifact_id": "revenue-forecast-q4",
                    "artifact_kind": "DataForecast",
                    "artifact_title": "Q4 Revenue Forecast",
                    "fragments": [{"id": "frag-003", "level": 0}],
                },
            }
        
        mcp_tools.resolve_ikam_references = mock_resolve
        
        try:
            # Commit with mocked DB connection
            commit_result = commit_sequence(
                sequencer_fragment_dict=create_result["sequencer_fragment"],
                committed_by="pm",
                sequencer_request_id="req-ikam-db-001",
                connection=mock_connection,
            )
            
            assert commit_result["status"] == "committed"
            assert commit_result["ikam_resolution"]["status"] == "resolved"
            assert commit_result["ikam_resolution"]["references_processed"] == 2
            assert commit_result["ikam_resolution"]["derivation_events_emitted"] == 2
            
            # Verify derivation events were created
            events = commit_result["derivation_events"]
            assert len(events) == 2
            
            # Check first event (cost-model reference)
            event1 = events[0]
            assert event1["source_id"] == "cost-model-v1"
            assert event1["derivation_type"] == "referenced_by_plan"
            assert event1["transformation"] == "planning.uses_variable"
            assert event1["transformation_params"]["reference_type"] == "uses_variable"
            
            # Check second event (revenue-forecast reference)
            event2 = events[1]
            assert event2["source_id"] == "revenue-forecast-q4"
            assert event2["derivation_type"] == "referenced_by_plan"
            assert event2["transformation"] == "planning.depends_on_formula"
            
            # Verify summary includes IKAM references count
            assert commit_result["summary"]["ikam_references"] == 2
            
        finally:
            # Restore original function
            mcp_tools.resolve_ikam_references = original_resolve


class TestMCPToolRegistry:
    """Test MCP tool registry configuration."""

    def test_mcp_tools_registry_has_all_tools(self):
        """Test that MCP_TOOLS registry contains all three tools."""
        assert "create_sequence" in MCP_TOOLS
        assert "validate_sequence" in MCP_TOOLS
        assert "commit_sequence" in MCP_TOOLS

    def test_create_sequence_schema(self):
        """Test create_sequence tool schema."""
        tool = MCP_TOOLS["create_sequence"]
        assert tool["name"] == "create_sequence"
        assert "description" in tool
        assert "inputSchema" in tool
        
        schema = tool["inputSchema"]
        assert schema["type"] == "object"
        assert "instruction" in schema["properties"]
        assert "phases" in schema["properties"]
        assert "dependencies" in schema["properties"]

    def test_validate_sequence_schema(self):
        """Test validate_sequence tool schema."""
        tool = MCP_TOOLS["validate_sequence"]
        assert tool["name"] == "validate_sequence"
        assert "sequencer_fragment" in tool["inputSchema"]["properties"]

    def test_commit_sequence_schema(self):
        """Test commit_sequence tool schema."""
        tool = MCP_TOOLS["commit_sequence"]
        assert tool["name"] == "commit_sequence"
        
        schema = tool["inputSchema"]
        assert "sequencer_fragment" in schema["properties"]
        assert "committed_by" in schema["properties"]
        assert "sequencer_request_id" in schema["properties"]
        assert len(schema["required"]) == 3


class TestEndToEndWorkflow:
    """Test complete end-to-end sequencer workflow."""

    def test_full_workflow_create_validate_commit(self, sample_phases, sample_dependencies):
        """Test complete workflow: create → validate → commit."""
        # Step 1: Create sequence
        create_result = create_sequence(
            instruction="Build MVP with requirements, design, and implementation",
            phases=sample_phases,
            dependencies=sample_dependencies,
            requested_by="product-manager",
            request_mode="medium",
            instruction_id="instruction-001",
        )

        assert create_result["status"] == "success"
        fragment_dict = create_result["sequencer_fragment"]

        # Step 2: Validate sequence
        validate_result = validate_sequence_tool(sequencer_fragment_dict=fragment_dict)

        assert validate_result["status"] == "valid"

        # Step 3: Commit sequence
        fragment_dict["id"] = "seq-fragment-mvp-001"  # Add ID for provenance
        commit_result = commit_sequence(
            sequencer_fragment_dict=fragment_dict,
            committed_by="project-manager",
            sequencer_request_id="request-mvp-001",
        )

        assert commit_result["status"] == "committed"
        
        # Verify provenance chain
        assert commit_result["provenance"]["derived_from"] == "seq-fragment-mvp-001"
        assert commit_result["provenance"]["sequencer_request_id"] == "request-mvp-001"

    def test_workflow_with_validation_failure(self, sample_phases):
        """Test workflow where validation fails and commit is rejected."""
        # Create invalid sequence
        invalid_deps = [
            {"predecessor_id": "phase-1", "successor_id": "phase-2", "edge_type": "phase"},
            {"predecessor_id": "phase-2", "successor_id": "phase-1", "edge_type": "phase"},
        ]

        create_result = create_sequence(
            instruction="Build feature",
            phases=sample_phases,
            dependencies=invalid_deps,
            requested_by="test-user",
            request_mode="simple",
        )

        assert create_result["status"] == "validation_failed"

        # Validate (should also fail)
        validate_result = validate_sequence_tool(
            sequencer_fragment_dict=create_result["sequencer_fragment"]
        )
        assert validate_result["status"] == "invalid"

        # Commit should be rejected
        commit_result = commit_sequence(
            sequencer_fragment_dict=create_result["sequencer_fragment"],
            committed_by="pm",
            sequencer_request_id="req-001",
        )
        assert commit_result["status"] == "error"
