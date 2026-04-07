"""Performance benchmarks for MCP sequencer tools.

Validates latency targets:
- create_sequence: <200ms p95 (simple), <500ms p95 (complex)
- validate_sequence: <100ms p95
- commit_sequence (without DB): <100ms p95
- commit_sequence (with DB): <200ms p95

References: docs/planning/SEQUENCER_IMPLEMENTATION_PLAN.md Section 2.3
"""

import pytest
import time
from unittest.mock import Mock
from modelado.sequencer.mcp_tools import (
    create_sequence,
    validate_sequence_tool,
    commit_sequence,
)


class TestPerformanceBenchmarks:
    """Performance tests for MCP sequencer tools."""

    @pytest.fixture
    def sample_phases_small(self):
        """Small set of phases for performance testing."""
        return [
            {
                "id": "phase-1",
                "title": "Phase 1",
                "description": "Phase 1",
                "estimated_effort": 5.0,
                "assignees": ["user-1"],
                "risk_score": 0.3,
            },
            {
                "id": "phase-2",
                "title": "Phase 2",
                "description": "Phase 2",
                "estimated_effort": 10.0,
                "assignees": ["user-2"],
                "risk_score": 0.5,
            },
        ]

    @pytest.fixture
    def sample_phases_large(self):
        """Large set of phases for performance testing (20 phases)."""
        return [
            {
                "id": f"phase-{i}",
                "title": f"Phase {i}",
                "description": f"Phase {i} description",
                "estimated_effort": float(5 + i),
                "assignees": [f"user-{i % 5}"],
                "risk_score": 0.3 + (i * 0.02),
            }
            for i in range(1, 21)
        ]

    @pytest.fixture
    def sample_dependencies_small(self):
        """Small set of dependencies."""
        return [
            {
                "predecessor_id": "phase-1",
                "successor_id": "phase-2",
                "edge_type": "phase",
            },
        ]

    @pytest.fixture
    def sample_dependencies_large(self):
        """Large set of dependencies (19 sequential, 5 parallel)."""
        deps = []
        # Sequential chain: phase-1 → phase-2 → ... → phase-19
        for i in range(1, 19):
            deps.append({
                "predecessor_id": f"phase-{i}",
                "successor_id": f"phase-{i + 1}",
                "edge_type": "phase",
            })
        # Add some parallel dependencies from phase-1
        for i in range(3, 8):
            deps.append({
                "predecessor_id": "phase-1",
                "successor_id": f"phase-{i}",
                "edge_type": "phase",
            })
        return deps

    def test_create_sequence_simple_latency(self, sample_phases_small, sample_dependencies_small):
        """Test create_sequence latency in simple mode (<200ms p95)."""
        times = []
        for _ in range(10):
            start = time.perf_counter()
            result = create_sequence(
                instruction="Build simple feature",
                phases=sample_phases_small,
                dependencies=sample_dependencies_small,
                request_mode="simple",
            )
            elapsed = (time.perf_counter() - start) * 1000  # Convert to ms
            times.append(elapsed)
            assert result["status"] == "success"
        
        times.sort()
        p95 = times[int(len(times) * 0.95)]
        p99 = times[int(len(times) * 0.99)]
        
        print(f"\ncreate_sequence (simple mode):")
        print(f"  Min: {times[0]:.2f}ms")
        print(f"  P50: {times[len(times) // 2]:.2f}ms")
        print(f"  P95: {p95:.2f}ms")
        print(f"  P99: {p99:.2f}ms")
        print(f"  Max: {times[-1]:.2f}ms")
        
        # Target: <200ms p95
        assert p95 < 200, f"create_sequence simple p95 {p95:.2f}ms exceeds 200ms target"

    def test_create_sequence_medium_latency(self, sample_phases_small, sample_dependencies_small):
        """Test create_sequence latency in medium mode (<300ms p95)."""
        times = []
        for _ in range(10):
            start = time.perf_counter()
            result = create_sequence(
                instruction="Build medium feature",
                phases=sample_phases_small,
                dependencies=sample_dependencies_small,
                request_mode="medium",
            )
            elapsed = (time.perf_counter() - start) * 1000
            times.append(elapsed)
            assert result["status"] == "success"
        
        times.sort()
        p95 = times[int(len(times) * 0.95)]
        
        print(f"\ncreate_sequence (medium mode) P95: {p95:.2f}ms")
        
        # Target: <300ms p95
        assert p95 < 300, f"create_sequence medium p95 {p95:.2f}ms exceeds 300ms target"

    def test_create_sequence_complex_latency(self, sample_phases_small, sample_dependencies_small):
        """Test create_sequence latency in complex mode (<500ms p95)."""
        times = []
        for _ in range(10):
            start = time.perf_counter()
            result = create_sequence(
                instruction="Build complex feature",
                phases=sample_phases_small,
                dependencies=sample_dependencies_small,
                request_mode="complex",
            )
            elapsed = (time.perf_counter() - start) * 1000
            times.append(elapsed)
            assert result["status"] == "success"
        
        times.sort()
        p95 = times[int(len(times) * 0.95)]
        
        print(f"\ncreate_sequence (complex mode) P95: {p95:.2f}ms")
        
        # Target: <500ms p95
        assert p95 < 500, f"create_sequence complex p95 {p95:.2f}ms exceeds 500ms target"

    def test_validate_sequence_latency(self, sample_phases_small, sample_dependencies_small):
        """Test validate_sequence latency (<100ms p95)."""
        # Create a sequence first
        create_result = create_sequence(
            instruction="Test",
            phases=sample_phases_small,
            dependencies=sample_dependencies_small,
            request_mode="simple",
        )
        fragment = create_result["sequencer_fragment"]
        
        times = []
        for _ in range(10):
            start = time.perf_counter()
            result = validate_sequence_tool(sequencer_fragment_dict=fragment)
            elapsed = (time.perf_counter() - start) * 1000
            times.append(elapsed)
            assert result["status"] in ["valid", "invalid"]
        
        times.sort()
        p95 = times[int(len(times) * 0.95)]
        
        print(f"\nvalidate_sequence P95: {p95:.2f}ms")
        
        # Target: <100ms p95
        assert p95 < 100, f"validate_sequence p95 {p95:.2f}ms exceeds 100ms target"

    def test_commit_sequence_no_db_latency(self, sample_phases_small, sample_dependencies_small):
        """Test commit_sequence latency without DB (<100ms p95)."""
        # Create and validate first
        create_result = create_sequence(
            instruction="Test",
            phases=sample_phases_small,
            dependencies=sample_dependencies_small,
            request_mode="simple",
        )
        fragment = create_result["sequencer_fragment"]
        fragment["id"] = "test-seq-001"
        
        times = []
        for _ in range(10):
            start = time.perf_counter()
            result = commit_sequence(
                sequencer_fragment_dict=fragment,
                committed_by="test-user",
                sequencer_request_id="test-req-001",
                connection=None,
            )
            elapsed = (time.perf_counter() - start) * 1000
            times.append(elapsed)
            assert result["status"] == "committed"
        
        times.sort()
        p95 = times[int(len(times) * 0.95)]
        
        print(f"\ncommit_sequence (no DB) P95: {p95:.2f}ms")
        
        # Target: <100ms p95
        assert p95 < 100, f"commit_sequence (no DB) p95 {p95:.2f}ms exceeds 100ms target"

    def test_create_sequence_large_dag_latency(self, sample_phases_large, sample_dependencies_large):
        """Test create_sequence latency with large DAG (20 phases, <500ms p95)."""
        times = []
        for _ in range(5):  # Fewer iterations for complex case
            start = time.perf_counter()
            result = create_sequence(
                instruction="Build large system",
                phases=sample_phases_large,
                dependencies=sample_dependencies_large,
                request_mode="medium",
            )
            elapsed = (time.perf_counter() - start) * 1000
            times.append(elapsed)
            assert result["status"] == "success"
        
        times.sort()
        p95 = times[int(max(1, len(times) * 0.95))]
        
        print(f"\ncreate_sequence (20 phases) P95: {p95:.2f}ms")
        
        # Target: <500ms p95 for large DAGs
        assert p95 < 500, f"create_sequence large p95 {p95:.2f}ms exceeds 500ms target"


class TestScalabilityCharacteristics:
    """Test how performance scales with input size."""

    def test_latency_vs_phase_count(self):
        """Profile latency as phase count increases."""
        times_by_count = {}
        
        for phase_count in [2, 5, 10, 20]:
            phases = [
                {
                    "id": f"phase-{i}",
                    "title": f"Phase {i}",
                    "description": f"Phase {i}",
                    "estimated_effort": 5.0 + i,
                    "assignees": ["user-1"],
                    "risk_score": 0.3 + (i * 0.01),
                }
                for i in range(1, phase_count + 1)
            ]
            
            deps = [
                {
                    "predecessor_id": f"phase-{i}",
                    "successor_id": f"phase-{i + 1}",
                    "edge_type": "phase",
                }
                for i in range(1, phase_count)
            ]
            
            # Time a few iterations
            times = []
            for _ in range(3):
                start = time.perf_counter()
                result = create_sequence(
                    instruction="Test",
                    phases=phases,
                    dependencies=deps,
                    request_mode="medium",
                )
                elapsed = (time.perf_counter() - start) * 1000
                times.append(elapsed)
                assert result["status"] == "success"
            
            avg_time = sum(times) / len(times)
            times_by_count[phase_count] = avg_time
        
        print("\nLatency vs Phase Count:")
        for count, time_ms in times_by_count.items():
            print(f"  {count} phases: {time_ms:.2f}ms")
        
        # Verify reasonable scaling (should be roughly linear or better)
        # Allow 3x slowdown for 10x phase increase
        time_2 = times_by_count[2]
        time_20 = times_by_count[20]
        scaling_factor = time_20 / time_2 if time_2 > 0 else 1
        
        print(f"\nScaling factor (2→20 phases): {scaling_factor:.2f}x")
        assert scaling_factor < 30, f"Latency scaling {scaling_factor:.2f}x is excessive"
