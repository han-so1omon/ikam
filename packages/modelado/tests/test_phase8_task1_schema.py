"""
Test Suite: Phase 8 Task 1 - PostgreSQL Schema Validation
Purpose: Verify all tables, indices, constraints, and views work correctly
Status: Phase 8 Task 1
"""

import pytest


pytest.skip(
    "Legacy IKAM concept-table schema (ikam_concepts, v_* views) has been replaced by the fragment-based "
    "schema (e.g., ikam_fragments, ikam_fragment_meta, ikam_artifacts). These assertions are no longer "
    "valid under the current database preseed.",
    allow_module_level=True,
)

import os
import uuid
from sqlalchemy import text, create_engine
from sqlalchemy.exc import IntegrityError

# Test database configuration
_raw_database_url = (
    os.getenv("TEST_DATABASE_URL")
    or os.getenv("PYTEST_DATABASE_URL")
    or "postgresql://user:pass@localhost:5432/app"
)

# Prefer psycopg (v3) driver to avoid implicit psycopg2 dependency.
TEST_DATABASE_URL = (
    _raw_database_url.replace("postgresql://", "postgresql+psycopg://")
    if _raw_database_url.startswith("postgresql://")
    else _raw_database_url
)


@pytest.fixture(scope="session")
def db_engine():
    """Create database engine for testing"""
    engine = create_engine(TEST_DATABASE_URL, echo=False)
    yield engine
    engine.dispose()


@pytest.fixture
def db_conn(db_engine):
    """Create database connection for each test"""
    conn = db_engine.connect()
    yield conn
    conn.close()


class TestPhase8Task1SchemaCreation:
    """Test that all schema elements were created successfully"""

    def test_all_tables_exist(self, db_conn):
        """Verify all 6 ikam tables exist"""
        query = """
            SELECT table_name 
            FROM information_schema.tables 
            WHERE table_schema = 'public' AND table_name LIKE 'ikam_%'
            ORDER BY table_name
        """
        result = db_conn.execute(text(query))
        tables = [row[0] for row in result]
        
        expected_tables = {
            'ikam_concepts',
            'ikam_concept_maps',
            'ikam_concept_relationships',
            'ikam_extraction_batches',
            'ikam_concept_usage_audit',
            'ikam_concept_cache'
        }
        
        assert set(tables) == expected_tables, f"Missing tables: {expected_tables - set(tables)}"

    def test_ikam_concepts_columns(self, db_conn):
        """Verify ikam_concepts table has all required columns"""
        query = """
            SELECT column_name
            FROM information_schema.columns 
            WHERE table_name = 'ikam_concepts'
            ORDER BY column_name
        """
        result = db_conn.execute(text(query))
        columns = {row[0] for row in result}
        
        required_columns = {
            'id', 'concept_id', 'project_id', 'parent_concept_id', 'level',
            'title', 'summary', 'category', 'source_type', 'source_ids',
            'referenced_in_artifact_ids', 'referenced_in_conversation_ids',
            'referenced_in_plan_ids', 'confidence', 'salience',
            'created_at', 'updated_at', 'deleted_at', 'is_deleted', 'created_by'
        }
        
        missing = required_columns - columns
        assert not missing, f"Missing columns: {missing}"

    def test_all_indices_exist(self, db_conn):
        """Verify all indices were created"""
        query = """
            SELECT indexname 
            FROM pg_indexes 
            WHERE tablename LIKE 'ikam_%' AND schemaname = 'public'
            ORDER BY indexname
        """
        result = db_conn.execute(text(query))
        indices = [row[0] for row in result]
        
        assert len(indices) >= 20, f"Expected at least 20 indices, got {len(indices)}"
        assert any('project' in idx for idx in indices), "Missing project_id indices"

    def test_all_views_exist(self, db_conn):
        """Verify all 4 views were created"""
        query = """
            SELECT viewname 
            FROM pg_views 
            WHERE schemaname = 'public' AND viewname LIKE 'v_%'
            ORDER BY viewname
        """
        result = db_conn.execute(text(query))
        views = [row[0] for row in result]
        
        expected_views = {
            'v_concept_hierarchy',
            'v_concept_metrics',
            'v_extraction_stats',
            'v_root_concepts'
        }
        
        assert set(views) == expected_views, f"Missing views: {expected_views - set(views)}"


class TestPhase8Task1Constraints:
    """Test constraint validation"""

    def test_unique_concept_per_project(self, db_conn):
        """Verify UNIQUE(project_id, concept_id) constraint"""
        project_id = str(uuid.uuid4())
        
        # Insert first concept
        query1 = """
            INSERT INTO ikam_concepts 
            (concept_id, project_id, level, title, source_type)
            VALUES ('test-concept', :project_id, 0, 'Test', 'user_explicit')
        """
        db_conn.execute(text(query1), {"project_id": project_id})
        db_conn.commit()
        
        # Try to insert duplicate (should fail)
        query2 = """
            INSERT INTO ikam_concepts 
            (concept_id, project_id, level, title, source_type)
            VALUES ('test-concept', :project_id, 0, 'Test 2', 'user_explicit')
        """
        
        with pytest.raises(IntegrityError):
            db_conn.execute(text(query2), {"project_id": project_id})
            db_conn.commit()
        db_conn.rollback()

    def test_level_range_constraint(self, db_conn):
        """Verify level CHECK constraint (0-5)"""
        project_id = str(uuid.uuid4())
        
        # Try to insert level > 5 (should fail)
        query = """
            INSERT INTO ikam_concepts 
            (concept_id, project_id, level, title, source_type)
            VALUES ('test-level-6', :project_id, 6, 'Invalid', 'user_explicit')
        """
        
        with pytest.raises(IntegrityError):
            db_conn.execute(text(query), {"project_id": project_id})
            db_conn.commit()
        db_conn.rollback()


class TestPhase8Task1HierarchyValidation:
    """Test hierarchy validation and constraints"""

    def test_parent_level_less_than_child(self, db_conn):
        """Verify parent.level < child.level constraint"""
        project_id = str(uuid.uuid4())
        
        # Create parent at level 0
        parent_query = """
            INSERT INTO ikam_concepts 
            (concept_id, project_id, level, title, source_type)
            VALUES ('parent-concept', :project_id, 0, 'Parent', 'user_explicit')
            RETURNING id
        """
        result = db_conn.execute(text(parent_query), {"project_id": project_id})
        parent_id = result.scalar()
        db_conn.commit()
        
        # Create child at level 1 (should succeed)
        child_query = """
            INSERT INTO ikam_concepts 
            (concept_id, project_id, parent_concept_id, level, title, source_type)
            VALUES ('child-concept', :project_id, :parent_id, 1, 'Child', 'user_explicit')
        """
        db_conn.execute(
            text(child_query), 
            {"project_id": project_id, "parent_id": parent_id}
        )
        db_conn.commit()
        
        # Verify child was created
        verify_query = "SELECT COUNT(*) FROM ikam_concepts WHERE parent_concept_id = :parent_id"
        result = db_conn.execute(text(verify_query), {"parent_id": parent_id})
        count = result.scalar()
        assert count == 1, "Child concept should exist"


class TestPhase8Task1Hierarchy:
    """Test hierarchy queries and traversal"""

    def test_build_simple_hierarchy(self, db_conn):
        """Build a 3-level hierarchy and verify structure"""
        project_id = str(uuid.uuid4())
        
        # Level 0
        l0_query = """
            INSERT INTO ikam_concepts 
            (concept_id, project_id, level, title, source_type)
            VALUES ('l0', :project_id, 0, 'Level 0', 'user_explicit')
            RETURNING id
        """
        result = db_conn.execute(text(l0_query), {"project_id": project_id})
        l0_id = result.scalar()
        db_conn.commit()
        
        # Level 1
        l1_query = """
            INSERT INTO ikam_concepts 
            (concept_id, project_id, parent_concept_id, level, title, source_type)
            VALUES ('l1', :project_id, :parent_id, 1, 'Level 1', 'user_explicit')
            RETURNING id
        """
        result = db_conn.execute(
            text(l1_query),
            {"project_id": project_id, "parent_id": l0_id}
        )
        l1_id = result.scalar()
        db_conn.commit()
        
        # Level 2
        l2_query = """
            INSERT INTO ikam_concepts 
            (concept_id, project_id, parent_concept_id, level, title, source_type)
            VALUES ('l2', :project_id, :parent_id, 2, 'Level 2', 'user_explicit')
        """
        db_conn.execute(
            text(l2_query),
            {"project_id": project_id, "parent_id": l1_id}
        )
        db_conn.commit()
        
        # Verify hierarchy
        verify_query = """
            SELECT level, COUNT(*) as count
            FROM ikam_concepts
            WHERE project_id = :project_id
            GROUP BY level
            ORDER BY level
        """
        result = db_conn.execute(text(verify_query), {"project_id": project_id})
        levels = {row[0]: row[1] for row in result}
        
        assert levels[0] == 1, "Should have 1 concept at level 0"
        assert levels[1] == 1, "Should have 1 concept at level 1"
        assert levels[2] == 1, "Should have 1 concept at level 2"

    def test_recursive_hierarchy_query(self, db_conn):
        """Test recursive CTE to get all descendants"""
        project_id = str(uuid.uuid4())
        
        # Create a 4-level tree
        l0_query = "INSERT INTO ikam_concepts (concept_id, project_id, level, title, source_type) VALUES ('root', :pid, 0, 'Root', 'user_explicit') RETURNING id"
        result = db_conn.execute(text(l0_query), {"pid": project_id})
        l0_id = result.scalar()
        db_conn.commit()
        
        l1_query = "INSERT INTO ikam_concepts (concept_id, project_id, parent_concept_id, level, title, source_type) VALUES ('c1', :pid, :pid2, 1, 'Child1', 'user_explicit') RETURNING id"
        result = db_conn.execute(text(l1_query), {"pid": project_id, "pid2": l0_id})
        l1_id = result.scalar()
        db_conn.commit()
        
        l2_query = "INSERT INTO ikam_concepts (concept_id, project_id, parent_concept_id, level, title, source_type) VALUES ('c11', :pid, :pid2, 2, 'Child1.1', 'user_explicit') RETURNING id"
        result = db_conn.execute(text(l2_query), {"pid": project_id, "pid2": l1_id})
        l2_id = result.scalar()
        db_conn.commit()
        
        l3_query = "INSERT INTO ikam_concepts (concept_id, project_id, parent_concept_id, level, title, source_type) VALUES ('c111', :pid, :pid2, 3, 'Child1.1.1', 'user_explicit')"
        db_conn.execute(text(l3_query), {"pid": project_id, "pid2": l2_id})
        db_conn.commit()
        
        # Query descendants of root
        descendants_query = """
            WITH RECURSIVE descendants AS (
                SELECT id, concept_id, level FROM ikam_concepts WHERE id = :root_id
                UNION ALL
                SELECT c.id, c.concept_id, c.level 
                FROM ikam_concepts c
                INNER JOIN descendants d ON c.parent_concept_id = d.id
            )
            SELECT COUNT(*) FROM descendants
        """
        result = db_conn.execute(text(descendants_query), {"root_id": l0_id})
        count = result.scalar()
        
        assert count == 4, f"Should have 4 descendants (root + 3 children), got {count}"


class TestPhase8Task1Views:
    """Test view functionality"""

    def test_v_root_concepts_view(self, db_conn):
        """Test v_root_concepts view returns concepts with no parent"""
        project_id = str(uuid.uuid4())
        
        # Create root concept
        root_query = """
            INSERT INTO ikam_concepts 
            (concept_id, project_id, level, title, source_type)
            VALUES ('root1', :pid, 0, 'Root 1', 'user_explicit')
        """
        db_conn.execute(text(root_query), {"pid": project_id})
        db_conn.commit()
        
        # Query view
        view_query = "SELECT COUNT(*) FROM v_root_concepts WHERE project_id = :pid"
        result = db_conn.execute(text(view_query), {"pid": project_id})
        count = result.scalar()
        
        assert count >= 1, "View should return at least 1 root concept"

    def test_v_concept_metrics_view(self, db_conn):
        """Test v_concept_metrics view returns project statistics"""
        project_id = str(uuid.uuid4())
        
        # Create root at level 0
        root_query = """
            INSERT INTO ikam_concepts 
            (concept_id, project_id, level, title, source_type)
            VALUES ('root', :pid, 0, 'Root', 'user_explicit')
            RETURNING id
        """
        result = db_conn.execute(text(root_query), {"pid": project_id})
        root_id = result.scalar()
        db_conn.commit()
        
        # Create child at level 1
        child_query = """
            INSERT INTO ikam_concepts 
            (concept_id, project_id, parent_concept_id, level, title, source_type)
            VALUES ('c1', :pid, :parent_id, 1, 'Child 1', 'user_explicit')
            RETURNING id
        """
        result = db_conn.execute(text(child_query), {"pid": project_id, "parent_id": root_id})
        child_id = result.scalar()
        db_conn.commit()
        
        # Create grandchild at level 2
        grandchild_query = """
            INSERT INTO ikam_concepts 
            (concept_id, project_id, parent_concept_id, level, title, source_type)
            VALUES ('c11', :pid, :parent_id, 2, 'Child 1.1', 'user_explicit')
        """
        db_conn.execute(text(grandchild_query), {"pid": project_id, "parent_id": child_id})
        db_conn.commit()
        
        # Query metrics
        metrics_query = """
            SELECT total_concepts, root_concepts, max_depth 
            FROM v_concept_metrics 
            WHERE project_id = :pid
        """
        result = db_conn.execute(text(metrics_query), {"pid": project_id})
        metrics = result.first()
        
        assert metrics[0] == 3, "Should have 3 total concepts"
        assert metrics[1] == 1, "Should have 1 root concept"
        assert metrics[2] == 2, "Max depth should be 2"


class TestPhase8Task1SoftDelete:
    """Test soft delete functionality"""

    def test_soft_delete_concept(self, db_conn):
        """Verify soft delete works and doesn't break queries"""
        project_id = str(uuid.uuid4())
        
        # Create concept
        query = """
            INSERT INTO ikam_concepts 
            (concept_id, project_id, level, title, source_type)
            VALUES ('to-delete', :pid, 0, 'Will Delete', 'user_explicit')
            RETURNING id
        """
        result = db_conn.execute(text(query), {"pid": project_id})
        concept_id = result.scalar()
        db_conn.commit()
        
        # Soft delete
        delete_query = """
            UPDATE ikam_concepts 
            SET is_deleted = TRUE, deleted_at = NOW()
            WHERE id = :id
        """
        db_conn.execute(text(delete_query), {"id": concept_id})
        db_conn.commit()
        
        # Verify not in queries (WHERE NOT is_deleted)
        count_query = "SELECT COUNT(*) FROM ikam_concepts WHERE project_id = :pid AND NOT is_deleted"
        result = db_conn.execute(text(count_query), {"pid": project_id})
        count = result.scalar()
        assert count == 0, "Deleted concept should not appear in queries"


class TestPhase8Task1Performance:
    """Test query performance with indices"""

    def test_query_by_project_is_fast(self, db_conn):
        """Verify project_id queries are indexed"""
        project_id = str(uuid.uuid4())
        
        # Create test concepts
        for i in range(10):
            query = """
                INSERT INTO ikam_concepts 
                (concept_id, project_id, level, title, source_type)
                VALUES (:cid, :pid, 0, :title, 'user_explicit')
            """
            db_conn.execute(
                text(query),
                {"cid": f"perf-test-{i}", "pid": project_id, "title": f"Test {i}"}
            )
        db_conn.commit()
        
        # Query should be fast (with index)
        import time
        start = time.time()
        
        query = "SELECT COUNT(*) FROM ikam_concepts WHERE project_id = :pid"
        result = db_conn.execute(text(query), {"pid": project_id})
        count = result.scalar()
        
        elapsed = (time.time() - start) * 1000  # ms
        
        assert count == 10, "Should find 10 concepts"
        # Note: just checking it completes successfully, timing may vary in test env
        assert elapsed < 1000, f"Query should be reasonably fast (got {elapsed:.1f}ms)"


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
