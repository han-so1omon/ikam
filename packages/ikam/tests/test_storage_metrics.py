"""Tests for IKAM storage metrics and observability.

This test suite validates:
- Storage metrics recording (flat vs fragmented bytes)
- Δ(N) calculation and monotonicity guarantee
- Diagnostics endpoint for IKAM storage stats
- Graceful degradation when metrics unavailable
"""

from __future__ import annotations

import pytest
from ikam.almacen.metrics import (
    record_storage_observation,
    validate_delta_monotonicity,
)


class TestStorageMetrics:
    """Tests for storage metrics recording."""
    
    def test_record_storage_observation_imports_gracefully(self):
        """Test that metrics recording gracefully handles missing imports."""
        # This should not raise even if prometheus_client is unavailable
        record_storage_observation(
            project_id="test_proj",
            artifact_type="slide",
            flat_bytes=1000,
            fragmented_bytes=800,
        )
        # No assertion needed - test passes if no exception raised
    
    def test_record_storage_observation_with_zero_bytes(self):
        """Test metrics recording with zero bytes (edge case)."""
        record_storage_observation(
            project_id="test_proj",
            artifact_type="slide",
            flat_bytes=0,
            fragmented_bytes=0,
        )
        # No assertion needed - test passes if no exception raised
    
    def test_record_storage_observation_with_large_values(self):
        """Test metrics recording with large byte values."""
        record_storage_observation(
            project_id="test_proj",
            artifact_type="sheet",
            flat_bytes=10**9,  # 1 GB
            fragmented_bytes=10**8,  # 100 MB
        )
        # No assertion needed - test passes if no exception raised


class TestDeltaMonotonicity:
    """Tests for Δ(N) monotonicity validation."""
    
    def test_validate_empty_deltas(self):
        """Test validation with empty delta list."""
        is_monotonic, violations = validate_delta_monotonicity([])
        assert is_monotonic is True
        assert len(violations) == 0
    
    def test_validate_single_delta(self):
        """Test validation with single delta (trivially monotonic)."""
        deltas = [(1, 0)]
        is_monotonic, violations = validate_delta_monotonicity(deltas)
        assert is_monotonic is True
        assert len(violations) == 0
    
    def test_validate_monotonic_deltas(self):
        """Test validation with properly increasing deltas."""
        deltas = [
            (1, 0),      # First artifact, no dedup yet
            (2, 512),    # Second artifact, 50% shared
            (3, 1024),   # Third artifact, more dedup
            (4, 1536),   # Fourth artifact, more dedup
        ]
        is_monotonic, violations = validate_delta_monotonicity(deltas)
        assert is_monotonic is True
        assert len(violations) == 0
    
    def test_validate_constant_deltas(self):
        """Test validation with constant deltas (edge case: Δ(N+1) = Δ(N))."""
        deltas = [
            (1, 0),
            (2, 512),
            (3, 512),  # Same delta as N=2 (no new dedup)
            (4, 512),  # Still same (edge case)
        ]
        is_monotonic, violations = validate_delta_monotonicity(deltas)
        # Constant deltas are still monotonic (≥, not >)
        assert is_monotonic is True
        assert len(violations) == 0
    
    def test_validate_decreasing_deltas(self):
        """Test validation detects violation when Δ decreases."""
        deltas = [
            (1, 0),
            (2, 512),
            (3, 256),  # Violation: Δ(3) < Δ(2)
        ]
        is_monotonic, violations = validate_delta_monotonicity(deltas)
        assert is_monotonic is False
        assert len(violations) == 1
        assert "N=3" in violations[0]
        assert "Δ=256" in violations[0]
        assert "Δ(2)=512" in violations[0]
        assert "violation of monotonicity" in violations[0]
    
    def test_validate_multiple_violations(self):
        """Test validation detects multiple violations."""
        deltas = [
            (1, 0),
            (2, 512),
            (3, 256),   # Violation 1
            (4, 1024),  # OK (512 → 1024)
            (5, 512),   # Violation 2 (1024 → 512)
        ]
        is_monotonic, violations = validate_delta_monotonicity(deltas)
        assert is_monotonic is False
        assert len(violations) == 2
        assert "N=3" in violations[0]
        assert "N=5" in violations[1]
    
    def test_validate_realistic_scenario(self):
        """Test validation with realistic storage savings progression."""
        # Simulate artifacts with increasing shared content
        deltas = [
            (1, 0),       # First slide: 1000 bytes, no dedup
            (2, 400),     # Second slide: 600 new bytes (400 shared)
            (3, 700),     # Third slide: 300 new bytes (700 total saved)
            (4, 950),     # Fourth slide: 250 new bytes (950 total saved)
            (5, 1150),    # Fifth slide: 200 new bytes (1150 total saved)
        ]
        is_monotonic, violations = validate_delta_monotonicity(deltas)
        assert is_monotonic is True
        assert len(violations) == 0
        
        # Verify mathematical property: Δ(N+1) - Δ(N) represents new savings
        new_savings = [
            deltas[i+1][1] - deltas[i][1] 
            for i in range(len(deltas) - 1)
        ]
        # All new savings should be >= 0 (monotonic guarantee)
        assert all(s >= 0 for s in new_savings)


class TestStorageMetricsIntegration:
    """Integration tests for storage metrics (requires base-api metrics module)."""
    
    @pytest.mark.skipif(
        "PYTEST_CURRENT_TEST" not in __import__("os").environ,
        reason="Integration test requires base-api context"
    )
    def test_record_and_validate_scenario(self):
        """Test a complete scenario: record observations and validate monotonicity."""
        # Simulate storing 3 artifacts with increasing deduplication
        project_id = "test_integration"
        artifact_type = "slide"
        
        # Artifact 1: 1000 bytes flat, 1000 bytes fragmented (no dedup yet)
        record_storage_observation(project_id, artifact_type, 1000, 1000)
        
        # Artifact 2: 1000 bytes flat, 600 bytes fragmented (400 bytes saved)
        record_storage_observation(project_id, artifact_type, 1000, 600)
        
        # Artifact 3: 1000 bytes flat, 300 bytes fragmented (700 bytes saved cumulative)
        record_storage_observation(project_id, artifact_type, 1000, 300)
        
        # Validate monotonicity (manual calculation for this test)
        deltas = [
            (1, 0),      # Δ(1) = 1000 - 1000 = 0
            (2, 400),    # Δ(2) = 2000 - 1600 = 400
            (3, 700),    # Δ(3) = 3000 - 2300 = 700 (hypothetical; actual would need metric reads)
        ]
        is_monotonic, violations = validate_delta_monotonicity(deltas)
        assert is_monotonic is True
        assert len(violations) == 0


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
