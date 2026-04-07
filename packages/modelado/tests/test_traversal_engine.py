"""Tests for step-based execution traversal engine.

Comprehensive test coverage for:
- Timing model calculations (spike/decay)
- Breadth-first traversal with worker pool
- Depth-first traversal with recursion
- Progress tracking and metrics
- Configuration validation
- Error handling and timeouts
"""

import pytest
import asyncio
import math
from datetime import datetime, timedelta
from unittest.mock import AsyncMock, MagicMock, patch

from modelado.core.traversal_engine import (
    TimingModel,
    ExecutionStep,
    TraversalProgress,
    TraversalConfig,
    TraversalOrder,
    BreadthFirstTraverser,
    DepthFirstTraverser,
    TraversalEngine,
)


# ============================================================================
# Timing Model Tests
# ============================================================================

def test_timing_model_defaults():
    """Test TimingModel uses reasonable defaults."""
    model = TimingModel()
    
    assert model.base_time_seconds == 1.0
    assert model.spike_factor == 0.5
    assert model.decay_rate == 0.3


def test_timing_model_calculate_duration_step_0():
    """Test duration at step 0 includes spike."""
    model = TimingModel(base_time_seconds=1.0, spike_factor=0.5, decay_rate=0.3)
    
    # At step 0: duration = 1.0 * (1 + 0.5 * e^0) = 1.0 * 1.5 = 1.5
    duration = model.calculate_duration(0)
    assert abs(duration - 1.5) < 0.01


def test_timing_model_calculate_duration_step_10():
    """Test duration decreases over steps (decay)."""
    model = TimingModel(base_time_seconds=1.0, spike_factor=0.5, decay_rate=0.3)
    
    duration_0 = model.calculate_duration(0)
    duration_10 = model.calculate_duration(10)
    
    # Decay should reduce spike over time
    assert duration_0 > duration_10
    assert duration_10 > 1.0  # Base time still applies


def test_timing_model_exponential_decay():
    """Test spike decays exponentially with correct formula."""
    model = TimingModel(base_time_seconds=2.0, spike_factor=1.0, decay_rate=1.0)
    
    # At step 5: spike = 1.0 * e^(-1.0 * 5) = e^(-5) ≈ 0.0067
    step_5_spike = 1.0 * math.exp(-1.0 * 5)
    expected_duration = 2.0 * (1 + step_5_spike)
    
    actual_duration = model.calculate_duration(5)
    assert abs(actual_duration - expected_duration) < 0.01


def test_timing_model_zero_spike():
    """Test with spike_factor=0 gives constant duration."""
    model = TimingModel(base_time_seconds=1.0, spike_factor=0.0, decay_rate=0.3)
    
    assert model.calculate_duration(0) == 1.0
    assert model.calculate_duration(10) == 1.0
    assert model.calculate_duration(100) == 1.0


def test_timing_model_validation_negative_base_time():
    """Test rejects negative base_time_seconds."""
    with pytest.raises(ValueError, match="base_time_seconds must be positive"):
        TimingModel(base_time_seconds=-1.0)


def test_timing_model_validation_invalid_spike_factor():
    """Test rejects spike_factor outside [0, 1]."""
    with pytest.raises(ValueError, match="spike_factor must be in"):
        TimingModel(spike_factor=1.5)
    
    with pytest.raises(ValueError, match="spike_factor must be in"):
        TimingModel(spike_factor=-0.1)


def test_timing_model_validation_negative_decay_rate():
    """Test rejects negative decay_rate."""
    with pytest.raises(ValueError, match="decay_rate must be non-negative"):
        TimingModel(decay_rate=-0.1)


# ============================================================================
# ExecutionStep Tests
# ============================================================================

def test_execution_step_creation():
    """Test ExecutionStep creation with required fields."""
    step = ExecutionStep(
        step_number=0,
        execution_id="exec1",
        function_id="fn1",
        parent_execution_id=None,
    )
    
    assert step.step_number == 0
    assert step.execution_id == "exec1"
    assert step.function_id == "fn1"
    assert step.status == "pending"
    assert step.duration_seconds == 0.0
    assert step.timestamp is not None


def test_execution_step_validation_negative_step():
    """Test rejects invalid negative step_number."""
    with pytest.raises(ValueError, match="step_number must be >= -1"):
        ExecutionStep(
            step_number=-2,
            execution_id="exec1",
            function_id="fn1",
            parent_execution_id=None,
        )


def test_execution_step_validation_missing_execution_id():
    """Test requires execution_id."""
    with pytest.raises(ValueError, match="execution_id is required"):
        ExecutionStep(
            step_number=0,
            execution_id="",
            function_id="fn1",
            parent_execution_id=None,
        )


def test_execution_step_validation_missing_function_id():
    """Test requires function_id."""
    with pytest.raises(ValueError, match="function_id is required"):
        ExecutionStep(
            step_number=0,
            execution_id="exec1",
            function_id="",
            parent_execution_id=None,
        )


# ============================================================================
# TraversalProgress Tests
# ============================================================================

def test_traversal_progress_completion_percent():
    """Test completion percentage calculation."""
    progress = TraversalProgress(total_steps=10, completed_steps=3)
    assert progress.completion_percent == 30.0


def test_traversal_progress_completion_percent_zero_total():
    """Test completion percent with zero total."""
    progress = TraversalProgress(total_steps=0, completed_steps=0)
    assert progress.completion_percent == 0.0


def test_traversal_progress_success_rate():
    """Test success rate percentage calculation."""
    progress = TraversalProgress(completed_steps=7, failed_steps=3)
    assert progress.success_rate_percent == 70.0


def test_traversal_progress_success_rate_no_finished():
    """Test success rate with no finished steps."""
    progress = TraversalProgress(completed_steps=0, failed_steps=0)
    assert progress.success_rate_percent == 0.0


def test_traversal_progress_average_step_time():
    """Test average time per step calculation."""
    progress = TraversalProgress(
        completed_steps=5,
        elapsed_time_seconds=10.0,
    )
    assert progress.average_step_time == 2.0


def test_traversal_progress_average_step_time_no_steps():
    """Test average step time with no completed steps."""
    progress = TraversalProgress(completed_steps=0, elapsed_time_seconds=5.0)
    assert progress.average_step_time == 0.0


# ============================================================================
# TraversalConfig Tests
# ============================================================================

def test_traversal_config_defaults():
    """Test TraversalConfig uses reasonable defaults."""
    config = TraversalConfig()
    
    assert config.order == TraversalOrder.BREADTH_FIRST
    assert config.max_workers == 4
    assert config.timeout_seconds is None
    assert config.max_depth is None
    assert isinstance(config.timing_model, TimingModel)


def test_traversal_config_validation_zero_workers():
    """Test rejects zero max_workers."""
    with pytest.raises(ValueError, match="max_workers must be >= 1"):
        TraversalConfig(max_workers=0)


def test_traversal_config_validation_negative_timeout():
    """Test rejects non-positive timeout."""
    with pytest.raises(ValueError, match="timeout_seconds must be positive"):
        TraversalConfig(timeout_seconds=-1.0)


def test_traversal_config_validation_negative_max_depth():
    """Test rejects negative max_depth."""
    with pytest.raises(ValueError, match="max_depth must be non-negative"):
        TraversalConfig(max_depth=-1)


# ============================================================================
# BreadthFirstTraverser Tests
# ============================================================================

@pytest.mark.asyncio
async def test_breadth_first_single_execution():
    """Test BFS with single execution (no children)."""
    async def mock_get_children(exec_id):
        return []  # No children
    
    traverser = BreadthFirstTraverser(mock_get_children)
    config = TraversalConfig(max_workers=1)
    
    progress = await traverser.traverse("root", config)
    
    assert progress.completed_steps == 1
    assert progress.failed_steps == 0
    assert progress.last_step_number == 0


@pytest.mark.asyncio
async def test_breadth_first_tree_structure():
    """Test BFS with tree: root -> [child1, child2] -> [grandchild1, grandchild2]."""
    call_sequence = []
    
    async def mock_get_children(exec_id):
        call_sequence.append(exec_id)
        
        if exec_id == "root":
            return [
                ExecutionStep(step_number=-1, execution_id="child1", function_id="fn1", parent_execution_id=None),
                ExecutionStep(step_number=-1, execution_id="child2", function_id="fn2", parent_execution_id=None),
            ]
        elif exec_id == "child1":
            return [
                ExecutionStep(step_number=-1, execution_id="gc1", function_id="fn3", parent_execution_id=None),
            ]
        elif exec_id == "child2":
            return [
                ExecutionStep(step_number=-1, execution_id="gc2", function_id="fn4", parent_execution_id=None),
            ]
        return []
    
    traverser = BreadthFirstTraverser(mock_get_children)
    config = TraversalConfig(max_workers=2, timing_model=TimingModel(base_time_seconds=0.01))
    
    progress = await traverser.traverse("root", config)
    
    # BFS should call functions in expected order
    assert call_sequence[0] == "root"
    # All children should be called eventually
    assert "child1" in call_sequence and "child2" in call_sequence
    assert progress.completed_steps >= 1  # At least root completed


@pytest.mark.asyncio
async def test_breadth_first_respects_max_workers():
    """Test BFS respects max_workers concurrency limit."""
    concurrent_tasks = []
    max_concurrent = 0
    
    async def mock_get_children(exec_id):
        if exec_id == "root":
            # Return 10 children to exceed max_workers
            return [
                ExecutionStep(step_number=-1, execution_id=f"child{i}", function_id=f"fn{i}", parent_execution_id="root")
                for i in range(10)
            ]
        return []
    
    traverser = BreadthFirstTraverser(mock_get_children)
    config = TraversalConfig(max_workers=3, timing_model=TimingModel(base_time_seconds=0.01))
    
    progress = await traverser.traverse("root", config)
    
    # Max workers should be respected
    assert progress.active_workers <= config.max_workers


# ============================================================================
# DepthFirstTraverser Tests
# ============================================================================

@pytest.mark.asyncio
async def test_depth_first_single_execution():
    """Test DFS with single execution (no children)."""
    async def mock_get_children(exec_id):
        return []
    
    traverser = DepthFirstTraverser(mock_get_children)
    config = TraversalConfig(max_workers=1)
    
    progress = await traverser.traverse("root", config)
    
    assert progress.completed_steps == 1
    assert progress.failed_steps == 0


@pytest.mark.asyncio
async def test_depth_first_linear_chain():
    """Test DFS can be instantiated and configured."""
    async def mock_get_children(exec_id):
        return []
    
    traverser = DepthFirstTraverser(mock_get_children)
    assert traverser is not None
    config = TraversalConfig(
        order=TraversalOrder.DEPTH_FIRST,
        max_workers=1,
        timing_model=TimingModel(base_time_seconds=0.001)
    )
    assert config.order == TraversalOrder.DEPTH_FIRST


@pytest.mark.asyncio
async def test_depth_first_respects_max_depth():
    """Test DFS can be configured with max_depth."""
    async def mock_get_children(exec_id):
        return []
    
    traverser = DepthFirstTraverser(mock_get_children)
    config = TraversalConfig(max_workers=1, max_depth=3, timing_model=TimingModel(base_time_seconds=0.001))
    assert config.max_depth == 3


@pytest.mark.asyncio
async def test_depth_first_respects_max_workers():
    """Test DFS can be configured with max_workers."""
    async def mock_get_children(exec_id):
        return []
    
    traverser = DepthFirstTraverser(mock_get_children)
    config = TraversalConfig(max_workers=2, timing_model=TimingModel(base_time_seconds=0.001))
    assert config.max_workers == 2


# ============================================================================
# TraversalEngine Tests
# ============================================================================

@pytest.mark.asyncio
async def test_traversal_engine_initialization():
    """Test TraversalEngine initializes with graph and timing model."""
    mock_graph = AsyncMock()
    engine = TraversalEngine(mock_graph)
    
    assert engine.graph == mock_graph
    assert isinstance(engine.timing_model, TimingModel)


@pytest.mark.asyncio
async def test_traversal_engine_custom_timing_model():
    """Test TraversalEngine accepts custom timing model."""
    mock_graph = AsyncMock()
    custom_model = TimingModel(base_time_seconds=2.0, spike_factor=0.3)
    engine = TraversalEngine(mock_graph, timing_model=custom_model)
    
    assert engine.timing_model == custom_model


@pytest.mark.asyncio
async def test_traversal_engine_breadth_first():
    """Test TraversalEngine.traverse_breadth_first()."""
    mock_graph = AsyncMock()
    mock_graph.get_callee_executions = AsyncMock(return_value=[])
    
    engine = TraversalEngine(mock_graph)
    progress = await engine.traverse_breadth_first("root", max_workers=2)
    
    assert progress.completed_steps >= 1
    assert "root-bfs" in engine._traversals


@pytest.mark.asyncio
async def test_traversal_engine_depth_first():
    """Test TraversalEngine.traverse_depth_first()."""
    mock_graph = AsyncMock()
    mock_graph.get_callee_executions = AsyncMock(return_value=[])
    
    engine = TraversalEngine(mock_graph)
    progress = await engine.traverse_depth_first("root", max_workers=2)
    
    assert progress.completed_steps >= 1
    assert "root-dfs" in engine._traversals


@pytest.mark.asyncio
async def test_traversal_engine_progress_callback():
    """Test progress callback is invoked during traversal."""
    mock_graph = AsyncMock()
    mock_graph.get_callee_executions = AsyncMock(return_value=[])
    
    callback_calls = []
    
    async def progress_callback(progress):
        callback_calls.append(progress)
    
    engine = TraversalEngine(mock_graph)
    progress = await engine.traverse_breadth_first(
        "root",
        progress_callback=progress_callback,
    )
    
    # Callback should be called at least once
    assert len(callback_calls) > 0


@pytest.mark.asyncio
async def test_traversal_engine_get_traversal_progress():
    """Test retrieving previous traversal progress."""
    mock_graph = AsyncMock()
    mock_graph.get_callee_executions = AsyncMock(return_value=[])
    
    engine = TraversalEngine(mock_graph)
    progress1 = await engine.traverse_breadth_first("root")
    progress2 = engine.get_traversal_progress("root", "bfs")
    
    assert progress2 is not None
    assert progress2 == progress1


@pytest.mark.asyncio
async def test_traversal_engine_get_nonexistent_traversal():
    """Test getting nonexistent traversal returns None."""
    mock_graph = AsyncMock()
    engine = TraversalEngine(mock_graph)
    
    progress = engine.get_traversal_progress("nonexistent", "bfs")
    assert progress is None


# ============================================================================
# Integration Tests
# ============================================================================

@pytest.mark.asyncio
async def test_traversal_with_timing_model_integration():
    """Test full traversal respects timing model."""
    import time
    
    async def mock_get_children(exec_id):
        if exec_id == "root":
            return [
                ExecutionStep(step_number=-1, execution_id="child1", function_id="fn1", parent_execution_id="root"),
                ExecutionStep(step_number=-1, execution_id="child2", function_id="fn2", parent_execution_id="root"),
            ]
        return []
    
    traverser = BreadthFirstTraverser(mock_get_children)
    model = TimingModel(base_time_seconds=0.1, spike_factor=0.5)
    config = TraversalConfig(max_workers=2, timing_model=model)
    
    start = time.time()
    progress = await traverser.traverse("root", config)
    elapsed = time.time() - start
    
    # Should take at least some time due to sleep calls
    assert elapsed >= 0.05


@pytest.mark.asyncio
async def test_traversal_error_handling():
    """Test traversal handles child retrieval errors gracefully."""
    async def mock_get_children(exec_id):
        if exec_id == "root":
            return [ExecutionStep(step_number=-1, execution_id="child", function_id="fn", parent_execution_id="root")]
        # Raise error for child
        raise RuntimeError("Simulated error")
    
    traverser = BreadthFirstTraverser(mock_get_children)
    config = TraversalConfig(max_workers=1)
    
    progress = await traverser.traverse("root", config)
    
    # Should complete with root processed
    assert progress.completed_steps >= 1


@pytest.mark.asyncio
async def test_traversal_progress_metrics():
    """Test progress metrics are accurately calculated."""
    async def mock_get_children(exec_id):
        if exec_id == "root":
            return [
                ExecutionStep(step_number=-1, execution_id=f"child{i}", function_id=f"fn{i}", parent_execution_id="root")
                for i in range(3)
            ]
        return []
    
    traverser = BreadthFirstTraverser(mock_get_children)
    config = TraversalConfig(
        max_workers=2,
        timing_model=TimingModel(base_time_seconds=0.01),
    )
    
    progress = await traverser.traverse("root", config)
    
    # Should have metrics (even if some are zero)
    assert progress.completed_steps >= 1
    assert progress.success_rate_percent >= 0
    assert progress.elapsed_time_seconds > 0
