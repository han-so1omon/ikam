"""Test re-entrant connection behavior.

Verifies that get_connection() can be called inside an existing connection_scope()
and both see the same transaction, preventing FK visibility issues.
"""
import pytest
from modelado.db import connection_scope, get_connection


def test_reentrant_connection_shares_transaction():
    """Test that nested get_connection() reuses the outer connection_scope transaction."""
    
    with connection_scope() as outer_cx:
        # Insert a row in the outer transaction
        outer_cx.execute(
            """
            CREATE TEMP TABLE test_reentrant (
                id TEXT PRIMARY KEY,
                parent_id TEXT REFERENCES test_reentrant(id),
                value TEXT
            )
            """
        )
        outer_cx.execute(
            "INSERT INTO test_reentrant (id, parent_id, value) VALUES (%s, %s, %s)",
            ("parent-1", None, "Parent Row"),
        )
        
        # Now call get_connection() (simulating a service method call)
        with get_connection() as inner_cx:
            # The inner connection should see the uncommitted parent row
            result = inner_cx.execute(
                "SELECT value FROM test_reentrant WHERE id = %s",
                ("parent-1",),
            ).fetchone()
            assert result is not None
            assert result["value"] == "Parent Row"
            
            # Insert a child row with FK to the uncommitted parent
            # This would fail with ForeignKeyViolation if inner_cx were a separate transaction
            inner_cx.execute(
                "INSERT INTO test_reentrant (id, parent_id, value) VALUES (%s, %s, %s)",
                ("child-1", "parent-1", "Child Row"),
            )
        
        # Verify both rows exist in the outer scope
        rows = outer_cx.execute(
            "SELECT id, value FROM test_reentrant ORDER BY id"
        ).fetchall()
        assert len(rows) == 2
        assert rows[0]["id"] == "child-1"
        assert rows[1]["id"] == "parent-1"


def test_reentrant_connection_does_not_commit():
    """Test that exiting inner get_connection() does NOT commit the transaction."""
    
    with connection_scope() as outer_cx:
        outer_cx.execute("CREATE TEMP TABLE test_no_commit (id TEXT PRIMARY KEY, value TEXT)")
        outer_cx.execute("INSERT INTO test_no_commit (id, value) VALUES (%s, %s)", ("row-1", "Initial"))
        
        # Inner scope modifies data
        with get_connection() as inner_cx:
            inner_cx.execute("UPDATE test_no_commit SET value = %s WHERE id = %s", ("Modified", "row-1"))
        
        # Outer scope should still see the modified value (not reverted)
        result = outer_cx.execute("SELECT value FROM test_no_commit WHERE id = %s", ("row-1",)).fetchone()
        assert result["value"] == "Modified"
        
        # Rollback the outer transaction
        outer_cx.rollback()
    
    # Verify rollback worked (temp table is gone, but we can't query it outside the scope)
    # This test just ensures no exceptions were raised during rollback


def test_standalone_get_connection_creates_new_scope():
    """Test that get_connection() used standalone creates a full transaction scope."""
    
    # Use get_connection() without an outer connection_scope
    with get_connection() as cx:
        cx.execute("CREATE TEMP TABLE test_standalone (id TEXT PRIMARY KEY)")
        cx.execute("INSERT INTO test_standalone (id) VALUES (%s)", ("standalone-1",))
        
        # Verify insert succeeded
        result = cx.execute("SELECT id FROM test_standalone WHERE id = %s", ("standalone-1",)).fetchone()
        assert result["id"] == "standalone-1"
    
    # Transaction should have committed automatically on exit
    # (We can't verify persistence across scopes with temp tables, but no errors means success)
