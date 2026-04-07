"""Integration tests for ExecutionLinkGraph with policy enforcement.

This module tests the integration of InvocationPolicy with ExecutionLinkGraph,
validating that policy limits are correctly enforced during graph construction
and execution tracking.
"""

import pytest
import time
import asyncio
from datetime import datetime, timedelta
from modelado.core.execution_links import ExecutionLinkGraph
from modelado.core.invocation_policy import (
    InvocationPolicy,
    CONSERVATIVE_POLICY,
    TESTING_POLICY,
)


@pytest.fixture
def temp_db_path(tmp_path):
    """Provide a temporary database path."""
    return str(tmp_path / "test_graph.db")


@pytest.mark.asyncio
async def test_execution_graph_with_default_policy(temp_db_path):
    """Test ExecutionLinkGraph uses DEFAULT policy by default."""
    graph = ExecutionLinkGraph(temp_db_path)
    
    # Add links within default limits (depth=3, fan_out=10)
    await graph.add_link(
        caller_execution_id="root",
        callee_execution_id="child1",
        caller_function_id="root_fn",
        callee_function_id="child1_fn",
        invocation_order=0,
    )
    await graph.add_link(
        caller_execution_id="child1",
        callee_execution_id="child2",
        caller_function_id="child1_fn",
        callee_function_id="child2_fn",
        invocation_order=0,
    )
    await graph.add_link(
        caller_execution_id="child2",
        callee_execution_id="child3",
        caller_function_id="child2_fn",
        callee_function_id="child3_fn",
        invocation_order=0,
    )
    
    stats = graph.get_policy_statistics()
    assert stats["total_executions"] == 3
    assert stats["max_depth"] == 3
    assert stats["max_fan_out"] == 10


@pytest.mark.asyncio
async def test_execution_graph_with_custom_policy(temp_db_path):
    """Test ExecutionLinkGraph with custom policy."""
    policy = InvocationPolicy(max_depth=2, max_fan_out=2)
    graph = ExecutionLinkGraph(temp_db_path, policy=policy)
    
    # Add links within custom limits
    await graph.add_link(
        caller_execution_id="root",
        callee_execution_id="child1",
        caller_function_id="root_fn",
        callee_function_id="child1_fn",
        invocation_order=0,
    )
    await graph.add_link(
        caller_execution_id="root",
        callee_execution_id="child2",
        caller_function_id="root_fn",
        callee_function_id="child2_fn",
        invocation_order=1,
    )
    
    stats = graph.get_policy_statistics()
    assert stats["total_executions"] == 2
    assert stats["max_depth"] == 2
    assert stats["max_fan_out"] == 2


@pytest.mark.asyncio
async def test_execution_graph_depth_limit_enforcement(temp_db_path):
    """Test depth limit prevents excessive nesting."""
    policy = InvocationPolicy(max_depth=2, max_fan_out=10)
    graph = ExecutionLinkGraph(temp_db_path, policy=policy)
    
    # Build a chain: root -> child1 -> child2 (depth=2)
    await graph.add_link(
        caller_execution_id="root",
        callee_execution_id="child1",
        caller_function_id="root_fn",
        callee_function_id="child1_fn",
        invocation_order=0,
    )
    await graph.add_link(
        caller_execution_id="child1",
        callee_execution_id="child2",
        caller_function_id="child1_fn",
        callee_function_id="child2_fn",
        invocation_order=0,
    )
    
    # Attempt to add child3 (depth=3) should fail
    with pytest.raises(RuntimeError, match="depth limit exceeded"):
        await graph.add_link(
            caller_execution_id="child2",
            callee_execution_id="child3",
            caller_function_id="child2_fn",
            callee_function_id="child3_fn",
            invocation_order=0,
            enforce_policy=True,
        )


@pytest.mark.asyncio
async def test_execution_graph_fan_out_limit_enforcement(temp_db_path):
    """Test fan-out limit prevents excessive branching."""
    policy = InvocationPolicy(max_depth=3, max_fan_out=2)
    graph = ExecutionLinkGraph(temp_db_path, policy=policy)
    
    # Add 2 children to root (at fan-out limit)
    await graph.add_link(
        caller_execution_id="root",
        callee_execution_id="child1",
        caller_function_id="root_fn",
        callee_function_id="child1_fn",
        invocation_order=0,
    )
    await graph.add_link(
        caller_execution_id="root",
        callee_execution_id="child2",
        caller_function_id="root_fn",
        callee_function_id="child2_fn",
        invocation_order=1,
    )
    
    # Attempt to add third child should fail
    with pytest.raises(RuntimeError, match="fan-out limit exceeded"):
        await graph.add_link(
            caller_execution_id="root",
            callee_execution_id="child3",
            caller_function_id="root_fn",
            callee_function_id="child3_fn",
            invocation_order=2,
            enforce_policy=True,
        )


@pytest.mark.asyncio
async def test_execution_graph_cost_budget_enforcement(temp_db_path):
    """Test cost budget prevents excessive spending."""
    policy = InvocationPolicy(max_depth=5, max_fan_out=10, max_total_cost=10.0)
    graph = ExecutionLinkGraph(temp_db_path, policy=policy)
    
    # Add links with costs totaling 9.0 (within budget)
    await graph.add_link(
        caller_execution_id="root",
        callee_execution_id="child1",
        caller_function_id="root_fn",
        callee_function_id="child1_fn",
        invocation_order=0,
        cost=3.0,
        enforce_policy=True,
    )
    await graph.add_link(
        caller_execution_id="root",
        callee_execution_id="child2",
        caller_function_id="root_fn",
        callee_function_id="child2_fn",
        invocation_order=1,
        cost=3.0,
        enforce_policy=True,
    )
    await graph.add_link(
        caller_execution_id="root",
        callee_execution_id="child3",
        caller_function_id="root_fn",
        callee_function_id="child3_fn",
        invocation_order=2,
        cost=3.0,
        enforce_policy=True,
    )
    
    # Attempt to add link exceeding budget should fail
    with pytest.raises(RuntimeError, match="cost budget exceeded"):
        await graph.add_link(
            caller_execution_id="root",
            callee_execution_id="child4",
            caller_function_id="root_fn",
            callee_function_id="child4_fn",
            invocation_order=3,
            cost=2.0,
            enforce_policy=True,
        )


@pytest.mark.asyncio
async def test_execution_graph_execution_count_enforcement(temp_db_path):
    """Test execution count limit prevents runaway operations."""
    policy = InvocationPolicy(max_depth=5, max_fan_out=10, max_total_executions=3)
    graph = ExecutionLinkGraph(temp_db_path, policy=policy)
    
    # Add 3 links (at execution limit)
    await graph.add_link(
        caller_execution_id="root",
        callee_execution_id="child1",
        caller_function_id="root_fn",
        callee_function_id="child1_fn",
        invocation_order=0,
        enforce_policy=True,
    )
    await graph.add_link(
        caller_execution_id="root",
        callee_execution_id="child2",
        caller_function_id="root_fn",
        callee_function_id="child2_fn",
        invocation_order=1,
        enforce_policy=True,
    )
    await graph.add_link(
        caller_execution_id="root",
        callee_execution_id="child3",
        caller_function_id="root_fn",
        callee_function_id="child3_fn",
        invocation_order=2,
        enforce_policy=True,
    )
    
    # Attempt to add fourth link should fail
    with pytest.raises(RuntimeError, match="execution count exceeded"):
        await graph.add_link(
            caller_execution_id="root",
            callee_execution_id="child4",
            caller_function_id="root_fn",
            callee_function_id="child4_fn",
            invocation_order=3,
            enforce_policy=True,
        )


@pytest.mark.asyncio
async def test_execution_graph_time_limit_enforcement(temp_db_path):
    """Test time limit prevents long-running operations."""
    policy = InvocationPolicy(max_depth=5, max_fan_out=10, max_execution_time_seconds=1.0)
    graph = ExecutionLinkGraph(temp_db_path, policy=policy)
    
    # Start tracking time
    await graph.add_link(
        caller_execution_id="root",
        callee_execution_id="child1",
        caller_function_id="root_fn",
        callee_function_id="child1_fn",
        invocation_order=0,
        enforce_policy=True,
    )
    
    # Wait to exceed time limit
    await asyncio.sleep(1.5)
    
    # Attempt to add link after timeout should fail
    with pytest.raises(RuntimeError, match="execution time exceeded"):
        await graph.add_link(
            caller_execution_id="root",
            callee_execution_id="child2",
            caller_function_id="root_fn",
            callee_function_id="child2_fn",
            invocation_order=1,
            enforce_policy=True,
        )


@pytest.mark.asyncio
async def test_execution_graph_cycle_detection_disallowed(temp_db_path):
    """Test cycle detection prevents circular dependencies when disallowed."""
    policy = InvocationPolicy(max_depth=5, max_fan_out=10, allow_cycles=False)
    graph = ExecutionLinkGraph(temp_db_path, policy=policy)
    
    # Build a chain: root -> child1 -> child2
    await graph.add_link(
        caller_execution_id="root",
        callee_execution_id="child1",
        caller_function_id="root_fn",
        callee_function_id="child1_fn",
        invocation_order=0,
    )
    await graph.add_link(
        caller_execution_id="child1",
        callee_execution_id="child2",
        caller_function_id="child1_fn",
        callee_function_id="child2_fn",
        invocation_order=0,
    )
    
    # Attempt to create cycle: child2 -> root should fail
    with pytest.raises(RuntimeError, match="cycle detected"):
        await graph.add_link(
            caller_execution_id="child2",
            callee_execution_id="root",
            caller_function_id="child2_fn",
            callee_function_id="root_fn",
            invocation_order=0,
            enforce_policy=True,
        )


@pytest.mark.asyncio
async def test_execution_graph_cycle_detection_allowed(temp_db_path):
    """Test cycles are permitted when allow_cycles=True."""
    policy = InvocationPolicy(max_depth=5, max_fan_out=10, allow_cycles=True)
    graph = ExecutionLinkGraph(temp_db_path, policy=policy)
    
    # Build a chain: root -> child1 -> child2
    await graph.add_link(
        caller_execution_id="root",
        callee_execution_id="child1",
        caller_function_id="root_fn",
        callee_function_id="child1_fn",
        invocation_order=0,
    )
    await graph.add_link(
        caller_execution_id="child1",
        callee_execution_id="child2",
        caller_function_id="child1_fn",
        callee_function_id="child2_fn",
        invocation_order=0,
    )
    
    # Creating cycle: child2 -> root should succeed
    await graph.add_link(
        caller_execution_id="child2",
        callee_execution_id="root",
        caller_function_id="child2_fn",
        callee_function_id="root_fn",
        invocation_order=0,
        enforce_policy=True,
    )
    
    stats = graph.get_policy_statistics()
    assert stats["total_executions"] == 3


@pytest.mark.asyncio
async def test_execution_graph_policy_enforcement_optional(temp_db_path):
    """Test policy enforcement can be disabled per-link."""
    policy = InvocationPolicy(max_depth=1, max_fan_out=1)
    graph = ExecutionLinkGraph(temp_db_path, policy=policy)
    
    # Add link with enforcement (respects limits)
    await graph.add_link(
        caller_execution_id="root",
        callee_execution_id="child1",
        caller_function_id="root_fn",
        callee_function_id="child1_fn",
        invocation_order=0,
        enforce_policy=True,
    )
    
    # Add link without enforcement (bypasses limits)
    await graph.add_link(
        caller_execution_id="child1",
        callee_execution_id="child2",
        caller_function_id="child1_fn",
        callee_function_id="child2_fn",
        invocation_order=0,
        enforce_policy=False,
    )
    
    # Both links should be added
    stats = graph.get_policy_statistics()
    assert stats["total_executions"] >= 2


@pytest.mark.asyncio
async def test_execution_graph_statistics_tracking(temp_db_path):
    """Test policy statistics are accurately tracked."""
    policy = InvocationPolicy(max_depth=3, max_fan_out=5, max_total_cost=100.0)
    graph = ExecutionLinkGraph(temp_db_path, policy=policy)
    
    # Add links with costs
    await graph.add_link(
        caller_execution_id="root",
        callee_execution_id="child1",
        caller_function_id="root_fn",
        callee_function_id="child1_fn",
        invocation_order=0,
        cost=10.0,
        enforce_policy=True,
    )
    await graph.add_link(
        caller_execution_id="root",
        callee_execution_id="child2",
        caller_function_id="root_fn",
        callee_function_id="child2_fn",
        invocation_order=1,
        cost=20.0,
        enforce_policy=True,
    )
    await graph.add_link(
        caller_execution_id="child1",
        callee_execution_id="child3",
        caller_function_id="child1_fn",
        callee_function_id="child3_fn",
        invocation_order=0,
        cost=5.0,
        enforce_policy=True,
    )
    
    stats = graph.get_policy_statistics()
    assert stats["total_cost"] == 35.0
    assert stats["total_executions"] == 3
    assert stats["cost_utilization_percent"] == 35.0
    assert 0 <= stats["elapsed_time_seconds"] <= 5.0


@pytest.mark.asyncio
async def test_execution_graph_reset_policy_tracking(temp_db_path):
    """Test policy tracking can be reset between sessions."""
    policy = InvocationPolicy(max_depth=3, max_fan_out=5)
    graph = ExecutionLinkGraph(temp_db_path, policy=policy)
    
    # Add links in first session
    await graph.add_link(
        caller_execution_id="root",
        callee_execution_id="child1",
        caller_function_id="root_fn",
        callee_function_id="child1_fn",
        invocation_order=0,
        cost=10.0,
        enforce_policy=True,
    )
    await graph.add_link(
        caller_execution_id="root",
        callee_execution_id="child2",
        caller_function_id="root_fn",
        callee_function_id="child2_fn",
        invocation_order=1,
        cost=20.0,
        enforce_policy=True,
    )
    
    stats = graph.get_policy_statistics()
    assert stats["total_executions"] == 2
    assert stats["total_cost"] == 30.0
    
    # Reset tracking
    graph.reset_policy_tracking()
    
    # Add links in second session
    await graph.add_link(
        caller_execution_id="root",
        callee_execution_id="child3",
        caller_function_id="root_fn",
        callee_function_id="child3_fn",
        invocation_order=2,
        cost=5.0,
        enforce_policy=True,
    )
    
    stats = graph.get_policy_statistics()
    assert stats["total_executions"] == 1
    assert stats["total_cost"] == 5.0


@pytest.mark.asyncio
async def test_execution_graph_conservative_policy(temp_db_path):
    """Test CONSERVATIVE policy enforces tight limits."""
    graph = ExecutionLinkGraph(temp_db_path, policy=CONSERVATIVE_POLICY)
    
    # Build a chain: root -> child1 -> child2 (depth=2, at limit)
    await graph.add_link(
        caller_execution_id="root",
        callee_execution_id="child1",
        caller_function_id="root_fn",
        callee_function_id="child1_fn",
        invocation_order=0,
    )
    await graph.add_link(
        caller_execution_id="child1",
        callee_execution_id="child2",
        caller_function_id="child1_fn",
        callee_function_id="child2_fn",
        invocation_order=0,
    )
    
    # Attempt to exceed depth limit should fail
    with pytest.raises(RuntimeError, match="depth limit exceeded"):
        await graph.add_link(
            caller_execution_id="child2",
            callee_execution_id="child3",
            caller_function_id="child2_fn",
            callee_function_id="child3_fn",
            invocation_order=0,
            enforce_policy=True,
        )


@pytest.mark.asyncio
async def test_execution_graph_testing_policy(temp_db_path):
    """Test TESTING policy allows quick validation runs."""
    graph = ExecutionLinkGraph(temp_db_path, policy=TESTING_POLICY)
    
    # TESTING allows cycles
    await graph.add_link(
        caller_execution_id="root",
        callee_execution_id="child1",
        caller_function_id="root_fn",
        callee_function_id="child1_fn",
        invocation_order=0,
    )
    await graph.add_link(
        caller_execution_id="child1",
        callee_execution_id="root",
        caller_function_id="child1_fn",
        callee_function_id="root_fn",
        invocation_order=0,
        enforce_policy=True,
    )
    
    stats = graph.get_policy_statistics()
    assert stats["total_executions"] == 2


@pytest.mark.asyncio
async def test_execution_graph_zero_cost_executions(temp_db_path):
    """Test executions with zero cost don't affect budget."""
    policy = InvocationPolicy(max_depth=5, max_fan_out=10, max_total_cost=10.0)
    graph = ExecutionLinkGraph(temp_db_path, policy=policy)
    
    # Add many zero-cost links
    for i in range(5):
        await graph.add_link(
            caller_execution_id="root",
            callee_execution_id=f"child{i}",
            caller_function_id="root_fn",
            callee_function_id=f"child{i}_fn",
            invocation_order=i,
            cost=0.0,
            enforce_policy=True,
        )
    
    stats = graph.get_policy_statistics()
    assert stats["total_cost"] == 0.0
    assert stats["total_executions"] == 5

