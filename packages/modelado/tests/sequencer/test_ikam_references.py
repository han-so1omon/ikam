"""Unit tests for IKAM reference resolution and validation.

Tests cover:
- resolve_ikam_references: valid artifacts, missing artifacts, fragments
- validate_ikam_references: scope validation, missing artifacts, warnings
- lookup_artifact_by_semantic_match: exact match, partial match, fuzzy match

Database Schema Requirements:
- ikam_artifacts table with (id, kind, title)
- ikam_fragment_meta table with (fragment_id, artifact_id, level)
- ikam_artifact_fragments junction table
"""

import pytest
import psycopg
from typing import Any
from modelado.sequencer.ikam_references import (
    resolve_ikam_references,
    validate_ikam_references,
    lookup_artifact_by_semantic_match,
    ValidationError,
)


@pytest.fixture
def test_connection(tmp_path):
    """Create in-memory test database with IKAM schema."""
    # Use pytest's database fixture or mock connection
    # For unit tests, we'll use a real PostgreSQL connection
    # This assumes PYTEST_DATABASE_URL is set (see AGENTS.md)
    import os
    db_url = os.getenv("PYTEST_DATABASE_URL") or os.getenv(
        "DATABASE_URL",
        "postgresql://user:pass@localhost:5432/app",
    )
    
    with psycopg.connect(db_url) as conn:
        # vNext cutover: these tests rely on legacy derived fragment tables.
        # Skip in environments where those tables are absent.
        with conn.cursor() as cursor:
            cursor.execute(
                """
                SELECT 1
                FROM information_schema.tables
                WHERE table_schema = current_schema()
                  AND table_name IN ('ikam_fragment_meta', 'ikam_fragment_content', 'ikam_fragment_radicals')
                LIMIT 1
                """
            )
            if not cursor.fetchone():
                pytest.skip(
                    "Legacy fragment tables not present (vNext schema cutover): "
                    "ikam_fragment_meta/content/radicals"
                )

        # Setup: insert test data
        with conn.cursor() as cursor:
            # Create test fragments first (CAS storage)
            cursor.execute("""
                INSERT INTO ikam_fragments (id, mime_type, size, bytes, created_at)
                VALUES
                    ('frag-001', 'text/plain', 100, 'test data'::bytea, now()),
                    ('frag-002', 'text/plain', 200, 'test data'::bytea, now()),
                    ('frag-003', 'application/json', 300, 'test data'::bytea, now()),
                    ('frag-004', 'application/json', 400, 'test data'::bytea, now())
                ON CONFLICT (id) DO NOTHING
            """)
            
            # Create test artifacts
            cursor.execute("""
                INSERT INTO ikam_artifacts (id, kind, title, created_at)
                VALUES
                    ('11111111-1111-1111-1111-111111111111'::uuid, 'EconomicModel', 'SaaS Unit Economics Model v3', now()),
                    ('22222222-2222-2222-2222-222222222222'::uuid, 'Sheet', 'Q4 Revenue Forecast', now()),
                    ('33333333-3333-3333-3333-333333333333'::uuid, 'Document', 'Investor Deck Draft', now())
                ON CONFLICT (id) DO NOTHING
            """)
            
            # Associate fragments to artifacts (current fragment set)
            cursor.execute("""
                INSERT INTO ikam_artifact_fragments (artifact_id, fragment_id, position)
                VALUES
                    ('11111111-1111-1111-1111-111111111111'::uuid, 'frag-001', 0),
                    ('11111111-1111-1111-1111-111111111111'::uuid, 'frag-002', 1),
                    ('22222222-2222-2222-2222-222222222222'::uuid, 'frag-003', 0),
                    ('22222222-2222-2222-2222-222222222222'::uuid, 'frag-004', 1)
                ON CONFLICT (artifact_id, fragment_id) DO UPDATE SET
                    position = EXCLUDED.position
            """)
            
            conn.commit()
        
        yield conn
        
        # Teardown: clean test data
        with conn.cursor() as cursor:
            cursor.execute("""
                DELETE FROM ikam_artifact_fragments
                WHERE artifact_id IN (
                    '11111111-1111-1111-1111-111111111111'::uuid,
                    '22222222-2222-2222-2222-222222222222'::uuid,
                    '33333333-3333-3333-3333-333333333333'::uuid
                )
            """)
            cursor.execute("""
                DELETE FROM ikam_fragments
                WHERE id IN ('frag-001', 'frag-002', 'frag-003', 'frag-004')
            """)
            cursor.execute("""
                DELETE FROM ikam_artifacts
                WHERE id IN (
                    '11111111-1111-1111-1111-111111111111'::uuid,
                    '22222222-2222-2222-2222-222222222222'::uuid,
                    '33333333-3333-3333-3333-333333333333'::uuid
                )
            """)
            conn.commit()
            conn.commit()


def test_resolve_ikam_references_valid_artifacts(test_connection):
    """Test resolving valid IKAM artifacts with fragments."""
    artifact_ids = [
        "11111111-1111-1111-1111-111111111111",
        "22222222-2222-2222-2222-222222222222"
    ]
    
    resolved = resolve_ikam_references(artifact_ids, test_connection)
    
    assert len(resolved) == 2
    
    # Check first artifact (EconomicModel)
    artifact1 = resolved["11111111-1111-1111-1111-111111111111"]
    assert artifact1["status"] == "RESOLVED"
    assert artifact1["artifact_kind"] == "EconomicModel"
    assert artifact1["artifact_title"] == "SaaS Unit Economics Model v3"
    assert len(artifact1["fragments"]) == 2
    assert artifact1["fragments"][0]["id"] == "frag-001"
    assert artifact1["fragments"][0]["level"] == 0
    assert artifact1["fragments"][1]["id"] == "frag-002"
    assert artifact1["fragments"][1]["level"] == 1
    
    # Check second artifact (Sheet)
    artifact2 = resolved["22222222-2222-2222-2222-222222222222"]
    assert artifact2["status"] == "RESOLVED"
    assert artifact2["artifact_kind"] == "Sheet"
    assert len(artifact2["fragments"]) == 2


def test_resolve_ikam_references_missing_artifact(test_connection):
    """Test resolving non-existent artifact returns NOT_FOUND status."""
    artifact_ids = ["99999999-9999-9999-9999-999999999999"]
    
    resolved = resolve_ikam_references(artifact_ids, test_connection)
    
    assert len(resolved) == 1
    artifact = resolved["99999999-9999-9999-9999-999999999999"]
    assert artifact["status"] == "NOT_FOUND"
    assert "not found" in artifact["error"].lower()
    assert artifact["artifact_id"] == "99999999-9999-9999-9999-999999999999"


def test_resolve_ikam_references_mixed_valid_invalid(test_connection):
    """Test resolving mix of valid and invalid artifacts."""
    artifact_ids = [
        "11111111-1111-1111-1111-111111111111",  # valid
        "99999999-9999-9999-9999-999999999999",  # invalid
        "22222222-2222-2222-2222-222222222222"   # valid
    ]
    
    resolved = resolve_ikam_references(artifact_ids, test_connection)
    
    assert len(resolved) == 3
    assert resolved["11111111-1111-1111-1111-111111111111"]["status"] == "RESOLVED"
    assert resolved["99999999-9999-9999-9999-999999999999"]["status"] == "NOT_FOUND"
    assert resolved["22222222-2222-2222-2222-222222222222"]["status"] == "RESOLVED"


def test_resolve_ikam_references_empty_list(test_connection):
    """Test resolving empty artifact list returns empty dict."""
    resolved = resolve_ikam_references([], test_connection)
    assert resolved == {}


def test_resolve_ikam_references_artifact_without_fragments(test_connection):
    """Test artifact with no fragments (Document has no fragments in test data)."""
    artifact_ids = ["33333333-3333-3333-3333-333333333333"]
    
    resolved = resolve_ikam_references(artifact_ids, test_connection)
    
    assert len(resolved) == 1
    artifact = resolved["33333333-3333-3333-3333-333333333333"]
    assert artifact["status"] == "RESOLVED"
    assert artifact["artifact_kind"] == "Document"
    assert artifact["fragments"] == []  # No fragments


def test_validate_ikam_references_all_valid(test_connection):
    """Test validation passes when all artifacts are resolved."""
    resolved = {
        "11111111-1111-1111-1111-111111111111": {
            "status": "RESOLVED",
            "artifact_id": "11111111-1111-1111-1111-111111111111",
            "artifact_kind": "EconomicModel",
            "artifact_title": "Cost Model",
            "fragments": [{"id": "frag-001", "level": 0}]
        }
    }
    phases = ["phase-1", "phase-2"]
    
    errors = validate_ikam_references(resolved, phases)
    
    assert len(errors) == 0


def test_validate_ikam_references_missing_artifact(test_connection):
    """Test validation fails for missing artifacts."""
    resolved = {
        "99999999-9999-9999-9999-999999999999": {
            "status": "NOT_FOUND",
            "artifact_id": "99999999-9999-9999-9999-999999999999",
            "error": "Artifact not found"
        }
    }
    phases = ["phase-1"]
    
    errors = validate_ikam_references(resolved, phases)
    
    assert len(errors) == 1
    assert errors[0].severity == "error"
    assert errors[0].code == "MISSING_ARTIFACT"
    assert errors[0].artifact_id == "99999999-9999-9999-9999-999999999999"


def test_validate_ikam_references_no_fragments_warning(test_connection):
    """Test validation warns when artifact has no fragments."""
    resolved = {
        "11111111-1111-1111-1111-111111111111": {
            "status": "RESOLVED",
            "artifact_id": "11111111-1111-1111-1111-111111111111",
            "artifact_kind": "EconomicModel",
            "artifact_title": "Empty Model",
            "fragments": []  # No fragments
        }
    }
    phases = ["phase-1"]
    
    errors = validate_ikam_references(resolved, phases)
    
    assert len(errors) == 1
    assert errors[0].severity == "warning"
    assert errors[0].code == "NO_FRAGMENTS"
    assert "no fragments" in errors[0].message.lower()


def test_lookup_artifact_by_semantic_match_exact_match(test_connection):
    """Test exact match (case-insensitive) on artifact title."""
    artifact_id = lookup_artifact_by_semantic_match(
        mention="SaaS Unit Economics Model v3",
        kind="EconomicModel",
        connection=test_connection
    )
    
    assert artifact_id == "11111111-1111-1111-1111-111111111111"


def test_lookup_artifact_by_semantic_match_partial_match(test_connection):
    """Test partial match on artifact title."""
    artifact_id = lookup_artifact_by_semantic_match(
        mention="revenue forecast",
        kind="Sheet",
        connection=test_connection
    )
    
    assert artifact_id == "22222222-2222-2222-2222-222222222222"


def test_lookup_artifact_by_semantic_match_no_match(test_connection):
    """Test no match returns None."""
    artifact_id = lookup_artifact_by_semantic_match(
        mention="nonexistent model",
        kind="EconomicModel",
        connection=test_connection
    )
    
    assert artifact_id is None


def test_lookup_artifact_by_semantic_match_wrong_kind(test_connection):
    """Test matching with wrong kind returns None."""
    artifact_id = lookup_artifact_by_semantic_match(
        mention="SaaS Unit Economics Model v3",
        kind="Sheet",  # Wrong kind (should be EconomicModel)
        connection=test_connection
    )
    
    assert artifact_id is None


def test_lookup_artifact_by_semantic_match_case_insensitive(test_connection):
    """Test case-insensitive matching."""
    artifact_id = lookup_artifact_by_semantic_match(
        mention="SAAS UNIT ECONOMICS MODEL V3",
        kind="EconomicModel",
        connection=test_connection
    )
    
    assert artifact_id == "11111111-1111-1111-1111-111111111111"


def test_lookup_artifact_by_semantic_match_empty_inputs(test_connection):
    """Test empty inputs return None."""
    assert lookup_artifact_by_semantic_match("", "EconomicModel", test_connection) is None
    assert lookup_artifact_by_semantic_match("model", "", test_connection) is None
    assert lookup_artifact_by_semantic_match(" ", "EconomicModel", test_connection) is None
