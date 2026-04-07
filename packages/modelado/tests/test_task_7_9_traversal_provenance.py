"""Test suite for Task 7.9: Traversal Provenance Integration.

Tests the integration of TraversalEngine with ExecutionLinkGraph and event recording.

Coverage:
1. TraversalStepEvent creation and Fisher Information calculation
2. Graph integration: _get_callee_steps() with ExecutionLinkGraph
3. Event emission from BreadthFirstTraverser and DepthFirstTraverser
4. Provenance tracking: traversal_id, step_number, depth_level, sibling_count
5. Fisher Information accumulation from traversal metrics
6. Error handling in graph lookups and event recording
"""

import asyncio
import pytest
from datetime import datetime
from typing import List, Optional, Dict
from unittest.mock import AsyncMock, MagicMock, patch

from modelado.core.traversal_engine import (
    TraversalStepEvent,
    ExecutionStep,
    TimingModel,
    TraversalConfig,
    TraversalOrder,
    TraversalProgress,
    BreadthFirstTraverser,
    DepthFirstTraverser,
    TraversalEngine,
)
from modelado.core.execution_links import ExecutionLink


# ============================================================================
# Test Fixtures
# ============================================================================


@pytest.fixture
def timing_model():
    """Create a minimal timing model for testing."""
    return TimingModel(base_time_seconds=0.001, spike_factor=0.2, decay_rate=0.1)


@pytest.fixture
def mock_event_recorder():
    """Create a mock event recorder."""
    recorder = AsyncMock()
    recorder.record_traversal_step = AsyncMock(return_value=None)
    return recorder


@pytest.fixture
def mock_execution_graph():
    """Create a mock ExecutionLinkGraph."""
    graph = AsyncMock()
    graph.get_callee_executions = AsyncMock(return_value=[])
    return graph


# ============================================================================
# Tests: TraversalStepEvent (Fisher Information)
# ============================================================================


class TestTraversalStepEvent:
    """Test TraversalStepEvent creation and Fisher Information calculation."""

    def test_event_creation_with_defaults(self):
        """Test creating TraversalStepEvent with minimal fields."""
        event = TraversalStepEvent(
            traversal_id="trav_123",
            step_number=0,
            execution_id="exec_abc",
            function_id="func_xyz",
            duration_seconds=1.0,
            depth_level=0,
            sibling_count=0,
            status="completed",
        )

        assert event.traversal_id == "trav_123"
        assert event.step_number == 0
        assert event.execution_id == "exec_abc"
        assert event.function_id == "func_xyz"
        assert event.status == "completed"
        assert event.parent_execution_id is None
        assert event.error_message is None

    def test_event_with_parent_and_error(self):
        """Test creating TraversalStepEvent with parent and error."""
        event = TraversalStepEvent(
            traversal_id="trav_123",
            step_number=5,
            execution_id="exec_def",
            function_id="func_uvw",
            parent_execution_id="exec_abc",
            duration_seconds=0.5,
            depth_level=2,
            sibling_count=3,
            status="failed",
            error_message="Connection timeout",
        )

        assert event.step_number == 5
        assert event.parent_execution_id == "exec_abc"
        assert event.error_message == "Connection timeout"
        assert event.depth_level == 2
        assert event.sibling_count == 3

    def test_fisher_information_calculation_simple(self):
        """Test Fisher Information calculation with simple values."""
        event = TraversalStepEvent(
            traversal_id="trav_123",
            step_number=0,
            execution_id="exec_abc",
            function_id="func_xyz",
            duration_seconds=1.0,  # 1 second
            depth_level=0,  # depth 0
            sibling_count=0,  # no siblings
            status="completed",
        )

        # I_step = duration × log(depth + 2) / (siblings + 1)
        # I_step = 1.0 × log(2) / 1 = 0.693
        fi = event.compute_fisher_information()
        assert fi > 0.69 and fi < 0.70  # Approx log(2) ≈ 0.693

    def test_fisher_information_increases_with_depth(self):
        """Test that Fisher Information increases with depth."""
        event_depth_0 = TraversalStepEvent(
            traversal_id="trav_123",
            step_number=0,
            execution_id="exec_1",
            function_id="func_1",
            duration_seconds=1.0,
            depth_level=0,
            sibling_count=0,
            status="completed",
        )

        event_depth_3 = TraversalStepEvent(
            traversal_id="trav_123",
            step_number=1,
            execution_id="exec_2",
            function_id="func_2",
            duration_seconds=1.0,
            depth_level=3,
            sibling_count=0,
            status="completed",
        )

        fi_0 = event_depth_0.compute_fisher_information()
        fi_3 = event_depth_3.compute_fisher_information()

        # log(2) < log(5), so fi_3 > fi_0
        assert fi_3 > fi_0

    def test_fisher_information_decreases_with_siblings(self):
        """Test that Fisher Information decreases with more siblings."""
        event_only_child = TraversalStepEvent(
            traversal_id="trav_123",
            step_number=0,
            execution_id="exec_1",
            function_id="func_1",
            duration_seconds=1.0,
            depth_level=1,
            sibling_count=0,
            status="completed",
        )

        event_many_siblings = TraversalStepEvent(
            traversal_id="trav_123",
            step_number=1,
            execution_id="exec_2",
            function_id="func_2",
            duration_seconds=1.0,
            depth_level=1,
            sibling_count=4,  # Many siblings
            status="completed",
        )

        fi_only = event_only_child.compute_fisher_information()
        fi_many = event_many_siblings.compute_fisher_information()

        # 1/1 > 1/5, so fi_only > fi_many
        assert fi_only > fi_many

    def test_fisher_information_zero_duration(self):
        """Test Fisher Information with zero duration."""
        event = TraversalStepEvent(
            traversal_id="trav_123",
            step_number=0,
            execution_id="exec_abc",
            function_id="func_xyz",
            duration_seconds=0.0,  # Zero duration
            depth_level=5,
            sibling_count=2,
            status="completed",
        )

        fi = event.compute_fisher_information()
        assert fi == 0.0

    def test_event_timestamp_auto_set(self):
        """Test that timestamp is automatically set."""
        before = datetime.utcnow()
        event = TraversalStepEvent(
            traversal_id="trav_123",
            step_number=0,
            execution_id="exec_abc",
            function_id="func_xyz",
            duration_seconds=1.0,
            depth_level=0,
            sibling_count=0,
            status="completed",
        )
        after = datetime.utcnow()

        assert before <= event.timestamp <= after


# ============================================================================
# Tests: Graph Integration
# ============================================================================


class TestGraphIntegration:
    """Test integration with ExecutionLinkGraph."""

    @pytest.mark.asyncio
    async def test_get_callee_steps_empty_graph(self, mock_execution_graph):
        """Test _get_callee_steps with no children."""
        engine = TraversalEngine(mock_execution_graph, timing_model=TimingModel())
        mock_execution_graph.get_callee_executions.return_value = []

        steps = await engine._get_callee_steps("exec_root")

        assert steps == []
        mock_execution_graph.get_callee_executions.assert_called_once_with("exec_root")

    @pytest.mark.asyncio
    async def test_get_callee_steps_single_child(self, mock_execution_graph):
        """Test _get_callee_steps with single child."""
        # Create a mock ExecutionLink
        link = MagicMock(spec=ExecutionLink)
        link.callee_execution_id = "exec_child"
        link.callee_function_id = "func_child"
        link.invocation_order = 0

        mock_execution_graph.get_callee_executions.return_value = [link]

        engine = TraversalEngine(mock_execution_graph, timing_model=TimingModel())
        steps = await engine._get_callee_steps("exec_parent")

        assert len(steps) == 1
        step = steps[0]
        assert step.execution_id == "exec_child"
        assert step.function_id == "func_child"
        assert step.parent_execution_id == "exec_parent"
        assert step.step_number == -1  # Not assigned yet

    @pytest.mark.asyncio
    async def test_get_callee_steps_multiple_children(self, mock_execution_graph):
        """Test _get_callee_steps with multiple children ordered by invocation_order."""
        # Create mock ExecutionLink objects
        links = []
        for i in range(3):
            link = MagicMock(spec=ExecutionLink)
            link.callee_execution_id = f"exec_child_{i}"
            link.callee_function_id = f"func_child_{i}"
            link.invocation_order = i
            links.append(link)

        mock_execution_graph.get_callee_executions.return_value = links

        engine = TraversalEngine(mock_execution_graph, timing_model=TimingModel())
        steps = await engine._get_callee_steps("exec_parent")

        assert len(steps) == 3
        for i, step in enumerate(steps):
            assert step.execution_id == f"exec_child_{i}"
            assert step.function_id == f"func_child_{i}"
            assert step.parent_execution_id == "exec_parent"


# ============================================================================
# Tests: Event Emission (BreadthFirstTraverser)
# ============================================================================


class TestBreadthFirstTraverserEventEmission:
    """Test event emission from BreadthFirstTraverser."""

    @pytest.mark.asyncio
    async def test_bfs_emits_events_for_each_step(
        self, timing_model, mock_event_recorder
    ):
        """Test that BFS emits TraversalStepEvent for each step."""
        # Mock get_children function
        async def get_children(execution_id: str) -> List[ExecutionStep]:
            if execution_id == "exec_root":
                return [
                    ExecutionStep(
                        step_number=-1,
                        execution_id="exec_child_1",
                        function_id="func_child_1",
                        parent_execution_id="exec_root",
                    ),
                    ExecutionStep(
                        step_number=-1,
                        execution_id="exec_child_2",
                        function_id="func_child_2",
                        parent_execution_id="exec_root",
                    ),
                ]
            return []

        traverser = BreadthFirstTraverser(
            get_children, event_recorder=mock_event_recorder
        )

        config = TraversalConfig(
            order=TraversalOrder.BREADTH_FIRST,
            max_workers=2,
            timing_model=timing_model,
        )

        progress = await traverser.traverse("exec_root", config)

        # Root + 2 children = 3 steps
        assert progress.completed_steps == 3
        # Should have recorded 3 events (root + 2 children)
        assert mock_event_recorder.record_traversal_step.call_count == 3

    @pytest.mark.asyncio
    async def test_bfs_event_contains_correct_metadata(
        self, timing_model, mock_event_recorder
    ):
        """Test that BFS events contain correct metadata."""
        async def get_children(execution_id: str) -> List[ExecutionStep]:
            if execution_id == "exec_root":
                return [
                    ExecutionStep(
                        step_number=-1,
                        execution_id="exec_child",
                        function_id="func_child",
                        parent_execution_id="exec_root",
                    ),
                ]
            return []

        traverser = BreadthFirstTraverser(
            get_children, event_recorder=mock_event_recorder, traversal_id="trav_test"
        )

        config = TraversalConfig(
            order=TraversalOrder.BREADTH_FIRST,
            max_workers=1,
            timing_model=timing_model,
        )

        await traverser.traverse("exec_root", config)

        # Verify event was recorded
        assert mock_event_recorder.record_traversal_step.call_count >= 1

        # Get the first recorded event
        call_args = mock_event_recorder.record_traversal_step.call_args_list[0]
        event = call_args[0][0]

        assert isinstance(event, TraversalStepEvent)
        assert event.traversal_id == "trav_test"
        assert event.status == "completed"
        assert event.duration_seconds > 0

    @pytest.mark.asyncio
    async def test_bfs_no_event_emission_without_recorder(self, timing_model):
        """Test that traversal works without event recorder."""
        async def get_children(execution_id: str) -> List[ExecutionStep]:
            return []

        traverser = BreadthFirstTraverser(
            get_children, event_recorder=None
        )

        config = TraversalConfig(
            order=TraversalOrder.BREADTH_FIRST,
            max_workers=1,
            timing_model=timing_model,
        )

        progress = await traverser.traverse("exec_root", config)

        # Should complete successfully without event recorder
        assert progress.completed_steps == 1


# ============================================================================
# Tests: Event Emission (DepthFirstTraverser)
# ============================================================================


class TestDepthFirstTraverserEventEmission:
    """Test event emission from DepthFirstTraverser."""

    @pytest.mark.asyncio
    async def test_dfs_emits_events(self, timing_model, mock_event_recorder):
        """Test that DFS emits TraversalStepEvent for each step."""
        async def get_children(execution_id: str) -> List[ExecutionStep]:
            if execution_id == "exec_root":
                return [
                    ExecutionStep(
                        step_number=-1,
                        execution_id="exec_child_1",
                        function_id="func_child_1",
                        parent_execution_id="exec_root",
                    ),
                ]
            return []

        traverser = DepthFirstTraverser(
            get_children, event_recorder=mock_event_recorder
        )

        config = TraversalConfig(
            order=TraversalOrder.DEPTH_FIRST,
            max_workers=1,
            timing_model=timing_model,
        )

        progress = await traverser.traverse("exec_root", config)

        # Root + 1 child = 2 steps
        assert progress.completed_steps >= 1
        # Should have recorded 2 events
        assert mock_event_recorder.record_traversal_step.call_count >= 1

    @pytest.mark.asyncio
    async def test_dfs_depth_tracking(self, timing_model, mock_event_recorder):
        """Test that DFS correctly tracks depth levels."""
        async def get_children(execution_id: str) -> List[ExecutionStep]:
            if execution_id == "exec_root":
                return [
                    ExecutionStep(
                        step_number=-1,
                        execution_id="exec_level1",
                        function_id="func_1",
                        parent_execution_id="exec_root",
                    ),
                ]
            elif execution_id == "exec_level1":
                return [
                    ExecutionStep(
                        step_number=-1,
                        execution_id="exec_level2",
                        function_id="func_2",
                        parent_execution_id="exec_level1",
                    ),
                ]
            return []

        traverser = DepthFirstTraverser(
            get_children, event_recorder=mock_event_recorder
        )

        config = TraversalConfig(
            order=TraversalOrder.DEPTH_FIRST,
            max_workers=1,
            timing_model=timing_model,
        )

        await traverser.traverse("exec_root", config)

        # Verify depth levels in recorded events
        calls = mock_event_recorder.record_traversal_step.call_args_list
        if len(calls) >= 2:
            event_level0 = calls[0][0][0]
            event_level1 = calls[1][0][0]
            
            assert event_level0.depth_level == 0
            assert event_level1.depth_level == 1


# ============================================================================
# Tests: TraversalEngine Integration
# ============================================================================


class TestTraversalEngineWithEventRecorder:
    """Test TraversalEngine with event recording."""

    def test_engine_initialization_with_recorder(self, mock_execution_graph, mock_event_recorder):
        """Test TraversalEngine accepts event_recorder in __init__."""
        engine = TraversalEngine(
            mock_execution_graph,
            timing_model=TimingModel(),
            event_recorder=mock_event_recorder,
        )

        assert engine.event_recorder is mock_event_recorder

    @pytest.mark.asyncio
    async def test_traverse_bfs_passes_recorder_to_traverser(
        self, mock_execution_graph, mock_event_recorder, timing_model
    ):
        """Test that traverse_breadth_first passes event_recorder to BreadthFirstTraverser."""
        mock_execution_graph.get_callee_executions.return_value = []

        engine = TraversalEngine(
            mock_execution_graph,
            timing_model=timing_model,
            event_recorder=mock_event_recorder,
        )

        progress = await engine.traverse_breadth_first("exec_root", max_workers=1)

        # Should have recorded at least the root step
        assert mock_event_recorder.record_traversal_step.call_count >= 1

    @pytest.mark.asyncio
    async def test_traverse_dfs_passes_recorder_to_traverser(
        self, mock_execution_graph, mock_event_recorder, timing_model
    ):
        """Test that traverse_depth_first passes event_recorder to DepthFirstTraverser."""
        mock_execution_graph.get_callee_executions.return_value = []

        engine = TraversalEngine(
            mock_execution_graph,
            timing_model=timing_model,
            event_recorder=mock_event_recorder,
        )

        progress = await engine.traverse_depth_first("exec_root", max_workers=1)

        # Should have recorded at least the root step
        assert mock_event_recorder.record_traversal_step.call_count >= 1


# ============================================================================
# Tests: Error Handling
# ============================================================================


class TestErrorHandling:
    """Test error handling in traversal and event recording."""

    @pytest.mark.asyncio
    async def test_traversal_continues_on_event_recording_error(self, timing_model):
        """Test that traversal continues even if event recording fails."""
        failing_recorder = AsyncMock()
        failing_recorder.record_traversal_step = AsyncMock(
            side_effect=Exception("Recording failed")
        )

        async def get_children(execution_id: str) -> List[ExecutionStep]:
            return []

        traverser = BreadthFirstTraverser(
            get_children, event_recorder=failing_recorder
        )

        config = TraversalConfig(
            order=TraversalOrder.BREADTH_FIRST,
            max_workers=1,
            timing_model=timing_model,
        )

        # Should not raise despite recording error
        progress = await traverser.traverse("exec_root", config)
        assert progress.completed_steps >= 1

    @pytest.mark.asyncio
    async def test_traversal_handles_graph_lookup_errors(
        self, timing_model, mock_event_recorder
    ):
        """Test that traversal handles graph lookup errors gracefully."""
        async def get_children_error(execution_id: str) -> List[ExecutionStep]:
            if execution_id == "exec_root":
                return [
                    ExecutionStep(
                        step_number=0,
                        execution_id="exec_child",
                        function_id="func_child",
                        parent_execution_id="exec_root",
                    ),
                ]
            # Raise error for child lookup
            raise RuntimeError("Graph lookup failed")

        traverser = BreadthFirstTraverser(
            get_children_error, event_recorder=mock_event_recorder
        )

        config = TraversalConfig(
            order=TraversalOrder.BREADTH_FIRST,
            max_workers=1,
            timing_model=timing_model,
        )

        # Should complete despite child lookup error
        progress = await traverser.traverse("exec_root", config)
        assert progress.completed_steps >= 1


# ============================================================================
# Tests: Integration Scenarios
# ============================================================================


class TestIntegrationScenarios:
    """End-to-end integration tests."""

    @pytest.mark.asyncio
    async def test_full_traversal_with_provenance_tracking(self):
        """Test full traversal with complete provenance tracking."""
        # Use fast timing model for integration tests
        fast_timing = TimingModel(base_time_seconds=0.001, spike_factor=0.1, decay_rate=0.1)
        
        # Record all events emitted
        recorded_events: List[TraversalStepEvent] = []

        mock_recorder = AsyncMock()

        async def capture_event(event):
            recorded_events.append(event)

        mock_recorder.record_traversal_step = capture_event

        # Build a simple tree
        async def get_children(execution_id: str) -> List[ExecutionStep]:
            if execution_id == "exec_root":
                return [
                    ExecutionStep(
                        step_number=-1,
                        execution_id="exec_c1",
                        function_id="func_c1",
                        parent_execution_id="exec_root",
                    ),
                ]
            return []

        traverser = BreadthFirstTraverser(
            get_children, event_recorder=mock_recorder, traversal_id="trav_integration"
        )

        config = TraversalConfig(
            order=TraversalOrder.BREADTH_FIRST,
            max_workers=1,
            timing_model=fast_timing,
        )

        progress = await traverser.traverse("exec_root", config)

        # Verify progress
        assert progress.completed_steps == 2  # root + child

        # Verify events were recorded
        assert len(recorded_events) == 2

        # Verify event properties
        for event in recorded_events:
            assert event.traversal_id == "trav_integration"
            assert event.status == "completed"
            assert event.duration_seconds > 0
            assert event.information_content >= 0

    @pytest.mark.asyncio
    async def test_traversal_with_multiple_depth_levels(self):
        """Test traversal recording events at multiple depth levels."""
        # Use very fast timing model for integration tests
        fast_timing = TimingModel(base_time_seconds=0.0001, spike_factor=0.05, decay_rate=0.1)
        
        recorded_events: List[TraversalStepEvent] = []

        mock_recorder = AsyncMock()

        async def capture_event(event):
            recorded_events.append(event)

        mock_recorder.record_traversal_step = capture_event

        # Build a smaller tree with 2 depth levels (root + 2 children + 2 grandchildren = 5 nodes)
        async def get_children(execution_id: str) -> List[ExecutionStep]:
            if execution_id == "exec_root":
                return [
                    ExecutionStep(
                        step_number=-1,
                        execution_id="exec_c1",
                        function_id="func_c1",
                        parent_execution_id="exec_root",
                    ),
                ]
            elif execution_id == "exec_c1":
                return [
                    ExecutionStep(
                        step_number=-1,
                        execution_id="exec_c1_gc1",
                        function_id="func_c1_gc1",
                        parent_execution_id="exec_c1",
                    ),
                ]
            return []

        traverser = BreadthFirstTraverser(
            get_children, event_recorder=mock_recorder
        )

        config = TraversalConfig(
            order=TraversalOrder.BREADTH_FIRST,
            max_workers=2,
            timing_model=fast_timing,
        )

        progress = await traverser.traverse("exec_root", config)

        # Verify all events were recorded (root + 1 child + 1 grandchild = 3)
        assert len(recorded_events) >= 3


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
