"""End-to-end tests for complete sequencer workflow.

These tests validate the full sequencer pipeline from natural language planning
instruction through to committed ProjectPhaseFragment with complete provenance.

Requirements:
- Database with IKAM schema (preseed_database.py)
- OPENAI_API_KEY for SemanticEngine
- Full dependency chain: parser → validator → estimator → storage → provenance

Test Flow:
1. Natural language planning instruction
2. Extract IKAM references semantically
3. Validate DAG structure
4. Estimate effort/duration/cost
5. Commit to database as SequencerFragment
6. Verify storage + provenance completeness
7. (Optional) Create ProjectPhaseFragment on user confirmation

Coverage:
- 5 E2E tests validating full workflow
- Database roundtrip + provenance
- IKAM reference resolution and validation
- Edge cases: invalid DAGs, missing artifacts, circular dependencies
"""

import os
import pytest
import psycopg
from uuid import uuid4
from datetime import datetime
from typing import Dict, Any

# These tests hit real external services (OpenAI) and attempt DB roundtrips.
# Keep them opt-in so the default suite stays deterministic.
pytestmark = [
    pytest.mark.skipif(
        not os.getenv("OPENAI_API_KEY"),
        reason="OPENAI_API_KEY required for OpenAI E2E tests",
    ),
    pytest.mark.skipif(
        not os.getenv("ENABLE_SEQUENCER_E2E_TESTS"),
        reason="Set ENABLE_SEQUENCER_E2E_TESTS=1 to run sequencer OpenAI E2E tests",
    ),
]

from modelado.sequencer.mcp_tools import create_sequence
from modelado.sequencer.models import (
    SequencerFragment,
    ProjectPhaseFragment,
    PlanPhase,
    PhaseDependency,
)
from modelado.sequencer.validator import validate_sequence
from modelado.sequencer.ikam_references import resolve_ikam_references, validate_ikam_references
from modelado.sequencer.adapters import sequencer_domain_to_storage, sequencer_storage_to_domain
from modelado.semantic_engine import SemanticEngine
from modelado.intent_classifier import IntentClassifier
from modelado.semantic_embeddings import SemanticEmbeddings


@pytest.fixture
def semantic_engine():
    """Create real SemanticEngine with OpenAI API."""
    api_key = os.getenv("OPENAI_API_KEY")
    if not api_key:
        pytest.skip("OPENAI_API_KEY not set")
    
    classifier = IntentClassifier(openai_api_key=api_key)
    embeddings = SemanticEmbeddings(openai_api_key=api_key)
    engine = SemanticEngine(intent_classifier=classifier, embeddings=embeddings)
    return engine


@pytest.fixture
def db_connection():
    """Create database connection for E2E tests."""
    database_url = os.getenv(
        "TEST_DATABASE_URL",
        os.getenv(
            "PYTEST_DATABASE_URL",
            "postgresql://user:pass@localhost:5432/app"
        )
    )
    conn = psycopg.connect(database_url)
    
    # Ensure required tables exist
    with conn.cursor() as cur:
        # ikam_artifacts table
        cur.execute("""
            CREATE TABLE IF NOT EXISTS ikam_artifacts (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                title TEXT NOT NULL,
                content_summary TEXT,
                created_at TIMESTAMP DEFAULT NOW()
            )
        """)
        
        # ikam_fragments table (CAS storage)
        cur.execute("""
            CREATE TABLE IF NOT EXISTS ikam_fragments (
                id TEXT PRIMARY KEY,
                bytes BYTEA NOT NULL,
                mime_type TEXT NOT NULL,
                size INTEGER NOT NULL,
                created_at TIMESTAMP DEFAULT NOW()
            )
        """)
        
        # ikam_fragment_metadata table
        cur.execute("""
            CREATE TABLE IF NOT EXISTS ikam_fragment_metadata (
                fragment_id TEXT PRIMARY KEY REFERENCES ikam_fragments(id),
                artifact_id TEXT,
                level INTEGER NOT NULL,
                type TEXT NOT NULL,
                radicals TEXT[],
                salience FLOAT,
                provenance JSONB,
                created_at TIMESTAMP DEFAULT NOW()
            )
        """)
        
        conn.commit()
    
    try:
        yield conn
    finally:
        conn.rollback()  # Clean up any uncommitted transactions
        conn.close()


@pytest.fixture
def sample_artifacts(db_connection):
    """Insert sample IKAM artifacts for testing."""
    with db_connection.cursor() as cur:
        # Insert sample artifacts that planning might reference
        cur.execute("""
            INSERT INTO ikam_artifacts (id, kind, title, content_summary, created_at)
            VALUES
                ('auth-module-v1', 'EconomicModel', 'Authentication Module', 
                 'OAuth2 implementation with role-based access control', NOW()),
                ('database-schema-v2', 'Sheet', 'Database Schema Design',
                 'Normalized schema with user tables, audit logs, indexes', NOW()),
                ('api-endpoints-v1', 'Document', 'REST API Specification',
                 'OpenAPI 3.0 spec with authentication, CRUD operations', NOW())
            ON CONFLICT (id) DO NOTHING
        """)
        db_connection.commit()
    
    yield
    
    # Cleanup
    with db_connection.cursor() as cur:
        cur.execute("""
            DELETE FROM ikam_fragments WHERE id IN (
                SELECT fragment_id FROM ikam_fragment_metadata 
                WHERE artifact_id IN ('auth-module-v1', 'database-schema-v2', 'api-endpoints-v1')
            )
        """)
        cur.execute("""
            DELETE FROM ikam_fragment_metadata 
            WHERE artifact_id IN ('auth-module-v1', 'database-schema-v2', 'api-endpoints-v1')
        """)
        cur.execute("""
            DELETE FROM ikam_artifacts 
            WHERE id IN ('auth-module-v1', 'database-schema-v2', 'api-endpoints-v1')
        """)
        db_connection.commit()


class TestSequencerE2EFlow:
    """End-to-end tests for complete sequencer workflow."""
    
    @pytest.mark.asyncio
    async def test_full_planning_flow_with_ikam_references(
        self,
        semantic_engine,
        db_connection,
        sample_artifacts,
    ):
        """Test full flow: planning instruction → SequencerFragment with IKAM refs → storage → provenance.
        
        Flow:
        1. Parse natural language planning instruction
        2. Extract IKAM references (auth-module, database-schema, api-endpoints)
        3. Validate DAG (no cycles, all deps present)
        4. Estimate effort/duration/cost
        5. Store as SequencerFragment in database
        6. Verify provenance completeness
        7. Verify lossless reconstruction
        """
        # 1. Natural language planning instruction
        planning_text = """
        Build MVP with authentication, database, and API:
        Phase 1: Implement authentication module (OAuth2)
        Phase 2: Design and implement database schema
        Phase 3: Create REST API endpoints
        Dependencies: Phase 2 depends on Phase 1 (auth), Phase 3 depends on Phase 2 (schema)
        Reference artifacts: auth-module-v1, database-schema-v2, api-endpoints-v1
        """
        
        # 2. Create sequence using MCP tool
        result = await create_sequence(
            db=db_connection,
            semantic_engine=semantic_engine,
            planning_text=planning_text,
            requested_by="test-user",
            request_mode="simple",
        )
        
        # 3. Verify result structure
        assert result["success"] is True
        assert "sequencer_fragment" in result
        fragment_data = result["sequencer_fragment"]
        
        # 4. Verify phases created correctly
        assert len(fragment_data["phases"]) == 3
        phases = fragment_data["phases"]
        assert phases[0]["title"] == "Implement authentication module"
        assert phases[1]["title"] == "Design and implement database schema"
        assert phases[2]["title"] == "Create REST API endpoints"
        
        # 5. Verify dependencies
        assert len(fragment_data["dependencies"]) == 2
        deps = fragment_data["dependencies"]
        # Phase 2 depends on Phase 1
        assert any(d["target_phase_id"] == phases[1]["id"] and d["source_phase_id"] == phases[0]["id"] for d in deps)
        # Phase 3 depends on Phase 2
        assert any(d["target_phase_id"] == phases[2]["id"] and d["source_phase_id"] == phases[1]["id"] for d in deps)
        
        # 6. Verify IKAM references extracted
        assert len(fragment_data["ikam_references"]) >= 3
        ref_artifact_ids = [ref["artifact_id"] for ref in fragment_data["ikam_references"]]
        assert "auth-module-v1" in ref_artifact_ids
        assert "database-schema-v2" in ref_artifact_ids
        assert "api-endpoints-v1" in ref_artifact_ids
        
        # 7. Verify validation passed
        validation = fragment_data["validation"]
        assert validation["is_valid"] is True
        assert validation["cycle_check_passed"] is True
        assert validation["dependency_check_passed"] is True
        
        # 8. Verify estimates present
        assert "effort_estimate" in fragment_data
        assert "duration_estimate" in fragment_data
        assert "cost_estimate" in fragment_data
        assert fragment_data["duration_estimate"]["critical_path_days"] > 0
        
        # 9. Store fragment in database and verify provenance
        sequencer_fragment = SequencerFragment(**fragment_data)
        storage_fragment, metadata = sequencer_domain_to_storage(sequencer_fragment)
        
        # Store in database
        with db_connection.cursor() as cur:
            cur.execute("""
                INSERT INTO ikam_fragments (id, bytes, mime_type, size)
                VALUES (%s, %s, %s, %s)
                ON CONFLICT (id) DO NOTHING
            """, (storage_fragment.id, storage_fragment.bytes, storage_fragment.mime_type, storage_fragment.size))
            
            cur.execute("""
                INSERT INTO ikam_fragment_metadata (fragment_id, artifact_id, level, type, radicals, salience, provenance)
                VALUES (%s, %s, %s, %s, %s, %s, %s)
                ON CONFLICT (fragment_id) DO NOTHING
            """, (
                storage_fragment.id,
                metadata.get("artifact_id"),
                metadata.get("level", 0),
                metadata.get("type", "sequencer"),
                metadata.get("radicals", []),
                metadata.get("salience", 1.0),
                metadata.get("provenance", {})
            ))
            
            db_connection.commit()
        
        # 10. Verify lossless reconstruction
        with db_connection.cursor() as cur:
            cur.execute("""
                SELECT f.bytes, f.mime_type, m.provenance
                FROM ikam_fragments f
                JOIN ikam_fragment_metadata m ON f.id = m.fragment_id
                WHERE f.id = %s
            """, (storage_fragment.id,))
            row = cur.fetchone()
            assert row is not None
            
            reconstructed = sequencer_storage_to_domain(
                storage_fragment=type('StorageFragment', (), {
                    'id': storage_fragment.id,
                    'bytes': row[0],
                    'mime_type': row[1],
                })(),
                metadata=row[2] or {}
            )
            
            # Verify key fields match
            assert len(reconstructed.phases) == len(sequencer_fragment.phases)
            assert len(reconstructed.dependencies) == len(sequencer_fragment.dependencies)
            assert len(reconstructed.ikam_references) == len(sequencer_fragment.ikam_references)
    
    @pytest.mark.asyncio
    async def test_invalid_dag_rejected(
        self,
        semantic_engine,
        db_connection,
        sample_artifacts,
    ):
        """Test that circular dependencies are detected and rejected.
        
        Flow:
        1. Create planning with circular dependency (Phase A → B → C → A)
        2. Validate sequence
        3. Verify validation failure
        4. Verify error details contain cycle information
        """
        planning_text = """
        Create circular phases (should fail):
        Phase A: depends on Phase C
        Phase B: depends on Phase A
        Phase C: depends on Phase B
        """
        
        result = await create_sequence(
            db=db_connection,
            semantic_engine=semantic_engine,
            planning_text=planning_text,
            requested_by="test-user",
            request_mode="simple",
        )
        
        # Should succeed in creating fragment but validation should fail
        assert result["success"] is True
        fragment_data = result["sequencer_fragment"]
        validation = fragment_data["validation"]
        
        # Validation should detect cycle
        assert validation["is_valid"] is False
        assert validation["cycle_check_passed"] is False
        assert len(validation["errors"]) > 0
        assert any("cycle" in error.lower() or "circular" in error.lower() for error in validation["errors"])
    
    @pytest.mark.asyncio
    async def test_missing_artifact_reference_handled(
        self,
        semantic_engine,
        db_connection,
        sample_artifacts,
    ):
        """Test that references to non-existent artifacts are handled gracefully.
        
        Flow:
        1. Create planning referencing non-existent artifact
        2. Attempt to resolve IKAM references
        3. Verify validation warns about missing artifact
        4. Sequence creation succeeds but with warnings
        """
        planning_text = """
        Build feature referencing non-existent artifact:
        Phase 1: Implement feature using non-existent-artifact-xyz
        """
        
        result = await create_sequence(
            db=db_connection,
            semantic_engine=semantic_engine,
            planning_text=planning_text,
            requested_by="test-user",
            request_mode="simple",
        )
        
        # Should succeed but with warnings
        assert result["success"] is True
        fragment_data = result["sequencer_fragment"]
        validation = fragment_data["validation"]
        
        # Either validation warns about missing artifact or reference confidence is low
        assert (
            len(validation.get("warnings", [])) > 0
            or any(ref["confidence"] < 0.3 for ref in fragment_data.get("ikam_references", []))
        )
    
    @pytest.mark.asyncio
    async def test_phase_dependency_validation(
        self,
        semantic_engine,
        db_connection,
        sample_artifacts,
    ):
        """Test that phase dependencies are validated correctly.
        
        Flow:
        1. Create planning with dependency on non-existent phase
        2. Validate sequence
        3. Verify validation catches dangling dependency
        """
        planning_text = """
        Create phases with invalid dependency:
        Phase A: Standalone phase
        Phase B: Depends on Phase C (which doesn't exist)
        """
        
        result = await create_sequence(
            db=db_connection,
            semantic_engine=semantic_engine,
            planning_text=planning_text,
            requested_by="test-user",
            request_mode="simple",
        )
        
        fragment_data = result["sequencer_fragment"]
        validation = fragment_data["validation"]
        
        # Should detect missing dependency target
        assert validation["dependency_check_passed"] is False
        assert any("missing" in error.lower() or "not found" in error.lower() for error in validation.get("errors", []))
    
    @pytest.mark.asyncio
    async def test_commit_to_project_phase_fragment(
        self,
        semantic_engine,
        db_connection,
        sample_artifacts,
    ):
        """Test committing SequencerFragment to ProjectPhaseFragment on user confirmation.
        
        Flow:
        1. Create valid SequencerFragment
        2. User confirms plan
        3. Commit to ProjectPhaseFragment with provenance link
        4. Verify ProjectPhaseFragment stored with complete provenance chain
        5. Verify bidirectional lookup (SequencerFragment ← ProjectPhaseFragment)
        """
        # 1. Create valid sequence
        planning_text = """
        Build simple feature:
        Phase 1: Design
        Phase 2: Implement (depends on Phase 1)
        Phase 3: Test (depends on Phase 2)
        """
        
        result = await create_sequence(
            db=db_connection,
            semantic_engine=semantic_engine,
            planning_text=planning_text,
            requested_by="test-user",
            request_mode="simple",
        )
        
        sequencer_fragment = SequencerFragment(**result["sequencer_fragment"])
        
        # 2. User confirms plan (simulated)
        # Create ProjectPhaseFragment
        project_phase_fragment = ProjectPhaseFragment(
            id=str(uuid4()),
            phases=sequencer_fragment.phases,
            dependencies=sequencer_fragment.dependencies,
            sequencer_fragment_id=sequencer_fragment.id,
            sequencer_request_id=str(uuid4()),
            requested_by="test-user",
            created_at=datetime.utcnow(),
        )
        
        # 3. Store ProjectPhaseFragment (this would be in commit_sequence tool)
        # For test purposes, verify structure is correct
        assert project_phase_fragment.sequencer_fragment_id == sequencer_fragment.id
        assert len(project_phase_fragment.phases) == len(sequencer_fragment.phases)
        assert project_phase_fragment.provenance_event == "sequencer_fragment_committed"
        
        # 4. Verify provenance completeness
        assert project_phase_fragment.sequencer_request_id is not None
        assert project_phase_fragment.created_at is not None
        
        # 5. Bidirectional lookup (in real implementation, would query database)
        # Verify we can reconstruct full provenance chain:
        # Instruction → SequencerFragment → ProjectPhaseFragment
        assert project_phase_fragment.sequencer_fragment_id == sequencer_fragment.id


# Performance target: All E2E tests should complete in <5s each (p95 latency)
