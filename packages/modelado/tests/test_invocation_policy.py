"""Tests for invocation policy enforcement (Phase 9.7, Task 7.7)."""

from datetime import datetime, timedelta
from unittest.mock import MagicMock
import time

import pytest

from modelado.core.invocation_policy import (
    InvocationPolicy,
    PolicyEnforcer,
    PolicyViolation,
    DEFAULT_POLICY,
    CONSERVATIVE_POLICY,
    GENEROUS_POLICY,
    TESTING_POLICY,
)


def test_invocation_policy_defaults():
    """Default policy has reasonable limits."""
    policy = DEFAULT_POLICY
    
    assert policy.max_depth == 3
    assert policy.max_fan_out == 10
    assert policy.max_total_cost == 100.0
    assert policy.max_execution_time_seconds == 300.0
    assert policy.allow_cycles is False


def test_invocation_policy_validation():
    """Policy validates parameters on construction."""
    # Valid policy
    policy = InvocationPolicy(max_depth=5, max_fan_out=20, max_total_cost=500.0)
    assert policy.max_depth == 5
    
    # Invalid depth
    with pytest.raises(ValueError, match="max_depth must be ≥ 0"):
        InvocationPolicy(max_depth=-1)
    
    # Invalid fan-out
    with pytest.raises(ValueError, match="max_fan_out must be ≥ 1"):
        InvocationPolicy(max_fan_out=0)
    
    # Invalid cost
    with pytest.raises(ValueError, match="max_total_cost must be > 0"):
        InvocationPolicy(max_total_cost=-10.0)


def test_policy_estimate_max_executions_linear_chain():
    """Linear chain (fan-out=1) estimates correctly."""
    policy = InvocationPolicy(max_depth=5, max_fan_out=1)
    
    # Linear chain: 1 + 1 + 1 + 1 + 1 + 1 = 6
    assert policy.estimate_max_executions() == 6


def test_policy_estimate_max_executions_binary_tree():
    """Binary tree (fan-out=2) estimates correctly."""
    policy = InvocationPolicy(max_depth=3, max_fan_out=2)
    
    # Binary tree: 1 + 2 + 4 + 8 = 15
    # Formula: (2^4 - 1) / (2 - 1) = 15
    assert policy.estimate_max_executions() == 15


def test_policy_estimate_max_executions_wide_tree():
    """Wide tree (fan-out=5) estimates correctly."""
    policy = InvocationPolicy(max_depth=3, max_fan_out=5)
    
    # Formula: (5^4 - 1) / (5 - 1) = 624 / 4 = 156
    assert policy.estimate_max_executions() == 156


def test_policy_estimate_with_hard_cap():
    """Hard cap overrides geometric estimate."""
    policy = InvocationPolicy(max_depth=5, max_fan_out=10, max_total_executions=50)
    
    # Without cap: (10^6 - 1) / 9 = 111,111
    # With cap: 50
    assert policy.estimate_max_executions() == 50


def test_policy_enforcer_initialization():
    """PolicyEnforcer initializes with clean state."""
    policy = TESTING_POLICY
    enforcer = PolicyEnforcer(policy)
    
    assert enforcer.total_cost == 0.0
    assert enforcer.total_executions == 0
    assert enforcer.start_time is None
    assert len(enforcer.violations) == 0


def test_policy_enforcer_depth_violation():
    """Enforcer detects depth violations."""
    policy = InvocationPolicy(max_depth=2)
    enforcer = PolicyEnforcer(policy)
    
    # Depth 0, 1, 2 are OK
    assert enforcer.check_before_add_link("caller", 0, 0) is None
    assert enforcer.check_before_add_link("caller", 1, 0) is None
    assert enforcer.check_before_add_link("caller", 2, 0) is None
    
    # Depth 3 violates limit
    violation = enforcer.check_before_add_link("caller", 3, 0)
    assert violation is not None
    assert violation.violation_type == "max_depth"
    assert violation.current_value == 3.0
    assert violation.limit_value == 2.0


def test_policy_enforcer_fan_out_violation():
    """Enforcer detects fan-out violations."""
    policy = InvocationPolicy(max_fan_out=3)
    enforcer = PolicyEnforcer(policy)
    
    # Fan-out 0, 1, 2 are OK
    assert enforcer.check_before_add_link("caller", 1, 0) is None
    assert enforcer.check_before_add_link("caller", 1, 1) is None
    assert enforcer.check_before_add_link("caller", 1, 2) is None
    
    # Fan-out 3 violates limit (already has 3 children)
    violation = enforcer.check_before_add_link("caller", 1, 3)
    assert violation is not None
    assert violation.violation_type == "max_fan_out"
    assert violation.current_value == 4.0  # Would add 4th child
    assert violation.limit_value == 3.0


def test_policy_enforcer_cost_violation():
    """Enforcer detects cost budget violations."""
    policy = InvocationPolicy(max_total_cost=10.0)
    enforcer = PolicyEnforcer(policy)
    
    # Add $8 worth of executions
    enforcer.record_execution(cost=5.0)
    enforcer.record_execution(cost=3.0)
    assert enforcer.total_cost == 8.0
    
    # Adding $1.5 is OK
    assert enforcer.check_before_add_link("caller", 1, 0, new_callee_cost=1.5) is None
    
    # Adding $3 violates budget
    violation = enforcer.check_before_add_link("caller", 1, 0, new_callee_cost=3.0)
    assert violation is not None
    assert violation.violation_type == "max_cost"
    assert violation.current_value == 11.0
    assert violation.limit_value == 10.0


def test_policy_enforcer_execution_count_violation():
    """Enforcer detects execution count violations."""
    policy = InvocationPolicy(max_total_executions=5)
    enforcer = PolicyEnforcer(policy)
    
    # Add 4 executions
    for _ in range(4):
        enforcer.record_execution()
    
    assert enforcer.total_executions == 4
    assert enforcer.check_before_add_link("caller", 1, 0) is None
    
    # 5th execution OK
    enforcer.record_execution()
    assert enforcer.total_executions == 5
    
    # 6th execution violates
    violation = enforcer.check_before_add_link("caller", 1, 0)
    assert violation is not None
    assert violation.violation_type == "max_executions"


def test_policy_enforcer_time_violation():
    """Enforcer detects execution time violations."""
    policy = InvocationPolicy(max_execution_time_seconds=1.0)
    enforcer = PolicyEnforcer(policy)
    enforcer.start_tracking()
    
    # Immediately check: OK
    assert enforcer.check_before_add_link("caller", 1, 0) is None
    
    # Wait for time limit
    time.sleep(1.1)
    
    # Now violates
    violation = enforcer.check_before_add_link("caller", 1, 0)
    assert violation is not None
    assert violation.violation_type == "max_time"


def test_policy_enforcer_cycle_detection_no_cycle():
    """Enforcer allows acyclic graphs."""
    policy = InvocationPolicy(allow_cycles=False)
    enforcer = PolicyEnforcer(policy)
    
    # Simple tree: A → B, A → C
    graph = {
        "exec_A": ["exec_B", "exec_C"],
        "exec_B": [],
        "exec_C": [],
    }
    
    # Adding D as child of B is OK
    violation = enforcer.check_cycle("exec_B", "exec_D", graph)
    assert violation is None


def test_policy_enforcer_cycle_detection_with_cycle():
    """Enforcer detects cycles in call graph."""
    policy = InvocationPolicy(allow_cycles=False)
    enforcer = PolicyEnforcer(policy)
    
    # Graph: A → B → C
    graph = {
        "exec_A": ["exec_B"],
        "exec_B": ["exec_C"],
        "exec_C": [],
    }
    
    # Adding edge C → A creates cycle
    violation = enforcer.check_cycle("exec_C", "exec_A", graph)
    assert violation is not None
    assert violation.violation_type == "cycle_detected"
    assert "exec_C" in violation.message
    assert "exec_A" in violation.message


def test_policy_enforcer_cycle_allowed():
    """Enforcer allows cycles when policy permits."""
    policy = InvocationPolicy(allow_cycles=True)
    enforcer = PolicyEnforcer(policy)
    
    graph = {
        "exec_A": ["exec_B"],
        "exec_B": ["exec_C"],
        "exec_C": [],
    }
    
    # Cycle allowed
    violation = enforcer.check_cycle("exec_C", "exec_A", graph)
    assert violation is None


def test_policy_enforcer_get_statistics():
    """Statistics include all metrics and utilization."""
    policy = InvocationPolicy(
        max_depth=3,
        max_fan_out=5,
        max_total_cost=100.0,
        max_execution_time_seconds=60.0,
        max_total_executions=50,
    )
    enforcer = PolicyEnforcer(policy)
    enforcer.start_tracking()
    
    # Add some executions
    enforcer.record_execution(cost=20.0)
    enforcer.record_execution(cost=15.0)
    enforcer.record_execution(cost=10.0)
    
    stats = enforcer.get_statistics()
    
    assert stats["total_cost"] == 45.0
    assert stats["total_executions"] == 3
    assert stats["violations"] == 0
    assert stats["policy"]["max_depth"] == 3
    assert stats["policy"]["max_fan_out"] == 5
    assert stats["utilization"]["cost_used_pct"] == 45.0
    assert stats["utilization"]["executions_used_pct"] == 6.0  # 3/50 * 100


def test_policy_enforcer_violation_tracking():
    """Enforcer tracks all violations."""
    policy = InvocationPolicy(max_depth=1, max_fan_out=2, max_total_cost=5.0)
    enforcer = PolicyEnforcer(policy)
    
    # Trigger depth violation
    enforcer.check_before_add_link("caller", 2, 0)
    
    # Trigger fan-out violation
    enforcer.check_before_add_link("caller", 1, 2)
    
    # Trigger cost violation
    enforcer.record_execution(cost=6.0)
    enforcer.check_before_add_link("caller", 1, 0, new_callee_cost=1.0)
    
    assert len(enforcer.violations) == 3
    assert enforcer.violations[0].violation_type == "max_depth"
    assert enforcer.violations[1].violation_type == "max_fan_out"
    assert enforcer.violations[2].violation_type == "max_cost"


def test_conservative_policy():
    """Conservative policy has tight limits."""
    policy = CONSERVATIVE_POLICY
    
    assert policy.max_depth == 2
    assert policy.max_fan_out == 5
    assert policy.max_total_cost == 10.0
    assert policy.max_execution_time_seconds == 60.0
    
    # Estimate: (5^3 - 1) / 4 = 31
    assert policy.estimate_max_executions() == 31


def test_generous_policy():
    """Generous policy has relaxed limits."""
    policy = GENEROUS_POLICY
    
    assert policy.max_depth == 5
    assert policy.max_fan_out == 20
    assert policy.max_total_cost == 500.0
    assert policy.max_execution_time_seconds == 600.0


def test_testing_policy():
    """Testing policy has very tight limits for fast tests."""
    policy = TESTING_POLICY
    
    assert policy.max_depth == 2
    assert policy.max_fan_out == 3
    assert policy.max_total_cost == 1.0
    assert policy.max_execution_time_seconds == 10.0
    
    # Estimate: (3^3 - 1) / 2 = 13
    assert policy.estimate_max_executions() == 13


def test_policy_violation_timestamp():
    """PolicyViolation includes timestamp."""
    before = datetime.utcnow()
    violation = PolicyViolation(
        violation_type="max_depth",
        message="Test violation",
    )
    after = datetime.utcnow()
    
    assert before <= violation.timestamp <= after


def test_policy_enforcer_reset():
    """start_tracking() resets state."""
    policy = DEFAULT_POLICY
    enforcer = PolicyEnforcer(policy)
    
    # Add some executions and violations
    enforcer.record_execution(cost=10.0)
    enforcer.record_execution(cost=5.0)
    enforcer.check_before_add_link("caller", 10, 0)  # Depth violation
    
    assert enforcer.total_cost == 15.0
    assert enforcer.total_executions == 2
    assert len(enforcer.violations) == 1
    
    # Reset
    enforcer.start_tracking()
    
    assert enforcer.total_cost == 0.0
    assert enforcer.total_executions == 0
    assert len(enforcer.violations) == 0
    assert enforcer.start_time is not None
