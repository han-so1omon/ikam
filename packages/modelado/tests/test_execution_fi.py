"""Tests for execution graph Fisher Information calculations (Phase 9.7, Task 7.6).

Note: FI functions interact with the database (ExecutionLinkGraph).
These tests validate mathematical properties and API contracts.
Integration tests with real DB are in test_execution_links_integration.py.
"""

import math
from unittest.mock import MagicMock
from collections import Counter

import pytest

from modelado.core.fisher_information import (
    compute_flat_execution_fi,
    compute_linked_execution_fi,
    compute_execution_fi_uplift,
    validate_execution_fi_dominance,
)


@pytest.fixture
def mock_connection_pool():
    return MagicMock()


def _build_mock_connection(cursor: MagicMock) -> MagicMock:
    connection = MagicMock()
    connection.__enter__ = MagicMock(return_value=connection)
    connection.__exit__ = MagicMock(return_value=False)
    connection.cursor = MagicMock(return_value=cursor)
    connection.cursor.return_value.__enter__ = MagicMock(return_value=cursor)
    connection.cursor.return_value.__exit__ = MagicMock(return_value=False)
    return connection


def test_compute_flat_execution_fi_single_function(mock_connection_pool):
    """Single function repeated N times → high count FI, low diversity FI."""
    cursor = MagicMock()
    # Mock query returning 10 executions of same function
    cursor.fetchall.return_value = [("gfn_analyzer",)] * 10
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    execution_ids = [f"exec_{i}" for i in range(10)]
    fi = compute_flat_execution_fi(mock_connection_pool, execution_ids)

    # log(N+1) with N=10 → log(11) ≈ 2.398, entropy = 0 (single function)
    assert fi == pytest.approx(2.398, abs=0.01)


def test_compute_flat_execution_fi_diverse_functions(mock_connection_pool):
    """Multiple different functions → high diversity FI."""
    cursor = MagicMock()
    # Mock query returning 3 different functions (uniform distribution)
    cursor.fetchall.return_value = [
        ("gfn_analyzer",),
        ("gfn_validator",),
        ("gfn_orchestrator",),
    ]
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    execution_ids = ["exec_1", "exec_2", "exec_3"]
    fi = compute_flat_execution_fi(mock_connection_pool, execution_ids)

    # log(3) ≈ 1.099 (count), log(3) ≈ 1.099 (diversity for uniform dist)
    # Total ≈ 2.198
    assert fi > 2.0
    assert fi < 2.5


def test_compute_linked_execution_fi_returns_higher_than_flat(mock_connection_pool):
    """Linked FI should be ≥ flat FI (dominance property)."""
    cursor = MagicMock()
    
    # Mock for tree traversal: root → child_1, child_2
    cursor.fetchall.side_effect = [
        [  # Children of root
            ("link_1", "exec_root", "exec_child_1", "gfn_orchestrator", "gfn_analyzer", 0, '{}', None),
            ("link_2", "exec_root", "exec_child_2", "gfn_orchestrator", "gfn_validator", 1, '{}', None),
        ],
        [],  # Children of child_1
        [],  # Children of child_2
        # For flat FI calculation
        [("gfn_orchestrator",), ("gfn_analyzer",), ("gfn_validator",)],
    ]
    
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    fi_linked = compute_linked_execution_fi(mock_connection_pool, "exec_root")

    # Reset for flat FI
    cursor.fetchall.side_effect = [
        [("gfn_orchestrator",), ("gfn_analyzer",), ("gfn_validator",)],
    ]
    
    fi_flat = compute_flat_execution_fi(
        mock_connection_pool,
        ["exec_root", "exec_child_1", "exec_child_2"],
    )

    # Dominance: fi_linked ≥ fi_flat
    assert fi_linked >= fi_flat


def test_compute_execution_fi_uplift_returns_non_negative(mock_connection_pool):
    """FI uplift should always be ≥ 0 (mathematical guarantee)."""
    cursor = MagicMock()
    
    # compute_execution_fi_uplift() does:
    # 1) BFS to collect execution IDs (children fetch for root, then child)
    # 2) compute_flat_execution_fi() (function_id fetch)
    # 3) compute_linked_execution_fi() (children fetch for root, then child)
    # 4) compute_flat_execution_fi() again inside linked (function_id fetch)
    cursor.fetchall.side_effect = [
        # (1) Collect execution IDs
        [("link_1", "exec_root", "exec_child", "gfn_orchestrator", "gfn_analyzer", 0, "{}", None)],
        [],
        # (2) Flat FI function IDs
        [("gfn_orchestrator",), ("gfn_analyzer",)],
        # (3) Linked traversal
        [("link_1", "exec_root", "exec_child", "gfn_orchestrator", "gfn_analyzer", 0, "{}", None)],
        [],
        # (4) Linked flat FI function IDs
        [("gfn_orchestrator",), ("gfn_analyzer",)],
    ]
    
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    result = compute_execution_fi_uplift(mock_connection_pool, "exec_root")

    assert result["n_executions"] >= 0
    assert result["fi_flat"] >= 0
    assert result["fi_linked"] >= result["fi_flat"]
    assert result["fi_uplift"] >= -1e-6  # Numerical tolerance
    assert result["uplift_ratio"] >= 1.0


def test_validate_execution_fi_dominance_passes_on_valid_graph(mock_connection_pool):
    """Validation passes when FI dominance holds."""
    cursor = MagicMock()
    
    cursor.fetchall.side_effect = [
        # (1) Collect execution IDs
        [("link_1", "exec_root", "exec_child", "gfn_orchestrator", "gfn_analyzer", 0, "{}", None)],
        [],
        # (2) Flat FI function IDs
        [("gfn_orchestrator",), ("gfn_analyzer",)],
        # (3) Linked traversal
        [("link_1", "exec_root", "exec_child", "gfn_orchestrator", "gfn_analyzer", 0, "{}", None)],
        [],
        # (4) Linked flat FI function IDs
        [("gfn_orchestrator",), ("gfn_analyzer",)],
    ]
    
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    result = validate_execution_fi_dominance(mock_connection_pool, "exec_root")

    assert result["valid"] is True
    assert result["fi_uplift"] >= -1e-6


def test_compute_flat_execution_fi_empty_list_returns_zero(mock_connection_pool):
    """Empty execution list → zero FI."""
    cursor = MagicMock()
    cursor.fetchall.return_value = []
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    fi = compute_flat_execution_fi(mock_connection_pool, [])

    assert fi == 0.0


def test_compute_linked_execution_fi_with_context_adds_information(mock_connection_pool):
    """Context snapshots increase FI via avg context size component."""
    cursor = MagicMock()
    
    # With context
    cursor.fetchall.side_effect = [
        [("link_1", "exec_root", "exec_child", "gfn_orchestrator", "gfn_analyzer", 0, '{"threshold": 0.85}', None)],
        [],
        [("gfn_orchestrator",), ("gfn_analyzer",)],
    ]
    
    connection_with_context = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection_with_context

    fi_with_context = compute_linked_execution_fi(mock_connection_pool, "exec_root")

    # Without context
    cursor2 = MagicMock()
    cursor2.fetchall.side_effect = [
        [("link_1", "exec_root", "exec_child", "gfn_orchestrator", "gfn_analyzer", 0, '{}', None)],
        [],
        [("gfn_orchestrator",), ("gfn_analyzer",)],
    ]
    
    connection_without_context = _build_mock_connection(cursor2)
    mock_connection_pool.connection.return_value = connection_without_context

    fi_without_context = compute_linked_execution_fi(mock_connection_pool, "exec_root")

    # Context should add information
    # Note: This test may be fragile depending on implementation details
    # The key property is fi_with_context ≥ fi_without_context
    assert fi_with_context >= fi_without_context


def test_validate_execution_fi_dominance_includes_metadata(mock_connection_pool):
    """Validation result includes all FI components."""
    cursor = MagicMock()
    
    cursor.fetchall.side_effect = [
        # (1) Collect execution IDs
        [("link_1", "exec_root", "exec_child", "gfn_orchestrator", "gfn_analyzer", 0, "{}", None)],
        [],
        # (2) Flat FI function IDs
        [("gfn_orchestrator",), ("gfn_analyzer",)],
        # (3) Linked traversal
        [("link_1", "exec_root", "exec_child", "gfn_orchestrator", "gfn_analyzer", 0, "{}", None)],
        [],
        # (4) Linked flat FI function IDs
        [("gfn_orchestrator",), ("gfn_analyzer",)],
    ]
    
    connection = _build_mock_connection(cursor)
    mock_connection_pool.connection.return_value = connection

    result = validate_execution_fi_dominance(mock_connection_pool, "exec_root")

    assert "valid" in result
    assert "fi_flat" in result
    assert "fi_linked" in result
    assert "fi_uplift" in result
    assert "n_executions" in result
    assert "uplift_ratio" in result
