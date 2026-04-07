"""Tests for model call provenance (Phase 9.7, Task 7.2).

Tests:
- ModelCallProvenanceEvent creation and validation
- ModelCallTracker recording and statistics
- Cost aggregation and cache hit tracking
- Batch event recording
- Deterministic replay capability
"""

import pytest
from datetime import datetime
from modelado.core.model_call_client import ModelCallClient, ModelCallParams
from modelado.core.model_call_provenance import (
    ModelCallProvenanceEvent,
    ModelCallBatchProvenanceEvent,
    ModelCallTracker,
)


class TestModelCallProvenanceEvent:
    """Unit tests for ModelCallProvenanceEvent."""
    
    def test_create_model_call_provenance_event(self):
        """Test creating a model call provenance event."""
        event = ModelCallProvenanceEvent(
            model_name="gpt-4o-mini",
            model_provider="openai",
            prompt_hash="e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            seed=42,
            temperature=0.7,
            max_tokens=500,
            output_hash="d4f7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e4f",
            output_length=287,
            input_tokens=145,
            output_tokens=287,
            cost_input_usd=0.000215,
            cost_output_usd=0.000862,
            total_cost_usd=0.001077,
            latency_ms=1230.5,
            was_cached=False,
            function_id="gfn_abc123",
            execution_id="exec_xyz789",
        )
        
        assert event.model_name == "gpt-4o-mini"
        assert event.model_provider == "openai"
        assert event.seed == 42
        assert event.temperature == 0.7
        assert event.total_cost_usd == 0.001077
        assert event.latency_ms == 1230.5
        assert event.was_cached is False
        assert event.event_id.startswith("mcall_")
    
    def test_model_call_provenance_with_cache(self):
        """Test model call provenance when cached."""
        event = ModelCallProvenanceEvent(
            model_name="claude-haiku",
            model_provider="anthropic",
            prompt_hash="abc123",
            temperature=0.5,
            max_tokens=300,
            output_hash="def456",
            output_length=150,
            input_tokens=100,
            output_tokens=150,
            cost_input_usd=0.0,
            cost_output_usd=0.0,
            total_cost_usd=0.0,
            latency_ms=10.0,  # Cache hit latency
            was_cached=True,
            cache_key="claude-haiku_abc123_42",
            function_id="gfn_def456",
            execution_id="exec_abc789",
        )
        
        assert event.was_cached is True
        assert event.cache_key == "claude-haiku_abc123_42"
        assert event.latency_ms == 10.0
        assert event.total_cost_usd == 0.0
    
    def test_model_call_provenance_without_seed(self):
        """Test model call provenance without deterministic seed."""
        event = ModelCallProvenanceEvent(
            model_name="gpt-4o-mini",
            model_provider="openai",
            prompt_hash="xyz789",
            seed=None,  # No deterministic seed
            temperature=1.0,
            max_tokens=1000,
            output_hash="hash1",
            output_length=500,
            input_tokens=200,
            output_tokens=500,
            cost_input_usd=0.0003,
            cost_output_usd=0.0015,
            total_cost_usd=0.0018,
            latency_ms=800.0,
            was_cached=False,
            function_id="gfn_xyz",
            execution_id="exec_xyz",
        )
        
        assert event.seed is None
        assert event.cache_key is None
    
    def test_model_call_provenance_validation(self):
        """Test validation of model call provenance."""
        # Invalid: temperature out of range
        with pytest.raises(ValueError):
            ModelCallProvenanceEvent(
                model_name="gpt-4o-mini",
                model_provider="openai",
                prompt_hash="hash",
                temperature=2.5,  # Max is 2.0
                max_tokens=500,
                output_hash="hash",
                output_length=100,
                input_tokens=50,
                output_tokens=100,
                cost_input_usd=0.0,
                cost_output_usd=0.0,
                total_cost_usd=0.0,
                latency_ms=100.0,
                was_cached=False,
                function_id="gfn",
                execution_id="exec",
            )
        
        # Invalid: negative latency
        with pytest.raises(ValueError):
            ModelCallProvenanceEvent(
                model_name="gpt-4o-mini",
                model_provider="openai",
                prompt_hash="hash",
                temperature=0.7,
                max_tokens=500,
                output_hash="hash",
                output_length=100,
                input_tokens=50,
                output_tokens=100,
                cost_input_usd=0.0,
                cost_output_usd=0.0,
                total_cost_usd=0.0,
                latency_ms=-100.0,  # Invalid
                was_cached=False,
                function_id="gfn",
                execution_id="exec",
            )
    
    def test_model_call_provenance_with_error(self):
        """Test model call provenance with error."""
        event = ModelCallProvenanceEvent(
            model_name="gpt-4o-mini",
            model_provider="openai",
            prompt_hash="hash",
            temperature=0.7,
            max_tokens=500,
            output_hash="",
            output_length=0,
            input_tokens=100,
            output_tokens=0,
            cost_input_usd=0.00015,
            cost_output_usd=0.0,
            total_cost_usd=0.00015,
            latency_ms=5000.0,
            was_cached=False,
            function_id="gfn",
            execution_id="exec",
            error="API timeout after 5 seconds",
        )
        
        assert event.error == "API timeout after 5 seconds"
        assert event.output_length == 0


class TestModelCallBatchProvenanceEvent:
    """Unit tests for ModelCallBatchProvenanceEvent."""
    
    def test_create_batch_provenance_event(self):
        """Test creating a batch provenance event."""
        event = ModelCallBatchProvenanceEvent(
            batch_id="batch_123",
            model_name="gpt-4o-mini",
            batch_size=5,
            param_hash="hash123",
            total_cost_usd=0.0052,
            total_input_tokens=725,
            total_output_tokens=1435,
            cached_items=2,
            function_ids=["gfn_abc123", "gfn_def456"],
        )
        
        assert event.batch_id == "batch_123"
        assert event.model_name == "gpt-4o-mini"
        assert event.batch_size == 5
        assert event.cached_items == 2
        assert len(event.function_ids) == 2
        assert event.event_id.startswith("mbatch_")
    
    def test_batch_provenance_event_timestamps(self):
        """Test batch event timestamp handling."""
        submitted = datetime.utcnow()
        completed = datetime.utcnow()
        
        event = ModelCallBatchProvenanceEvent(
            batch_id="batch_456",
            model_name="claude-haiku",
            batch_size=3,
            param_hash="hash456",
            total_cost_usd=0.001,
            total_input_tokens=300,
            total_output_tokens=600,
            submitted_at=submitted,
            completed_at=completed,
        )
        
        assert event.submitted_at == submitted
        assert event.completed_at == completed


class TestModelCallTracker:
    """Unit tests for ModelCallTracker."""
    
    def test_create_tracker(self):
        """Test creating a model call tracker."""
        tracker = ModelCallTracker(tracker_id="tracking_123")
        
        assert tracker.tracker_id == "tracking_123"
        assert len(tracker.call_events) == 0
        assert len(tracker.batch_events) == 0
    
    def test_record_single_model_call(self):
        """Test recording a single model call."""
        tracker = ModelCallTracker(tracker_id="tracking_123")
        
        # Create a mock result
        from modelado.core.model_call_client import ModelCallResult
        result = ModelCallResult(
            output="Analysis of elasticity patterns...",
            cost_usd=0.001077,
            latency_ms=1230.5,
            model="gpt-4o-mini",
            seed=42,
            prompt_hash="e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            output_hash="d4f7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e4f",
            cached=False,
        )
        
        params = ModelCallParams(
            model="gpt-4o-mini",
            prompt="Analyze elasticity patterns",
            temperature=0.7,
            max_tokens=500,
            seed=42,
        )
        
        event = tracker.record_model_call(
            function_id="gfn_abc123",
            model_call_result=result,
            params=params,
            execution_id="exec_xyz789",
        )
        
        assert event.model_name == "gpt-4o-mini"
        assert event.seed == 42
        assert event.was_cached is False
        assert len(tracker.call_events) == 1
    
    def test_record_multiple_calls_with_stats(self):
        """Test recording multiple calls and getting stats."""
        tracker = ModelCallTracker(tracker_id="tracking_456")
        
        from modelado.core.model_call_client import ModelCallResult
        
        # First call
        result1 = ModelCallResult(
            output="Output 1",
            cost_usd=0.001,
            latency_ms=1000.0,
            model="gpt-4o-mini",
            seed=42,
            prompt_hash="hash1",
            output_hash="out1",
            cached=False,
        )
        params1 = ModelCallParams(
            model="gpt-4o-mini",
            prompt="Prompt 1",
            temperature=0.7,
            max_tokens=500,
        )
        tracker.record_model_call(
            function_id="gfn_1",
            model_call_result=result1,
            params=params1,
            execution_id="exec_1",
        )
        
        # Second call (cached)
        result2 = ModelCallResult(
            output="Output 2",
            cost_usd=0.0,
            latency_ms=10.0,
            model="gpt-4o-mini",
            seed=42,
            prompt_hash="hash1",  # Same prompt
            output_hash="out1",
            cached=True,
        )
        params2 = ModelCallParams(
            model="gpt-4o-mini",
            prompt="Prompt 1",  # Same prompt
            temperature=0.7,
            max_tokens=500,
        )
        tracker.record_model_call(
            function_id="gfn_1",
            model_call_result=result2,
            params=params2,
            execution_id="exec_1",
        )
        
        # Third call (different model)
        result3 = ModelCallResult(
            output="Output 3",
            cost_usd=0.0005,
            latency_ms=800.0,
            model="claude-haiku",
            seed=None,
            prompt_hash="hash2",
            output_hash="out3",
            cached=False,
        )
        params3 = ModelCallParams(
            model="claude-haiku",
            prompt="Prompt 2",
            temperature=0.5,
            max_tokens=300,
        )
        tracker.record_model_call(
            function_id="gfn_2",
            model_call_result=result3,
            params=params3,
            execution_id="exec_2",
        )
        
        # Check stats
        stats = tracker.get_stats()
        
        assert stats["total_calls"] == 3
        assert stats["cache_hits"] == 1
        assert stats["cache_hit_rate"] == pytest.approx(1/3, abs=0.01)
        assert stats["total_cost_usd"] == pytest.approx(0.0015, abs=0.0001)
        assert "gpt-4o-mini" in stats["models_used"]
        assert "claude-haiku" in stats["models_used"]
    
    def test_record_batch_events(self):
        """Test recording batch events."""
        tracker = ModelCallTracker(tracker_id="tracking_789")
        
        from modelado.core.model_call_client import ModelCallResult
        
        # Record some individual calls
        result = ModelCallResult(
            output="Output",
            cost_usd=0.001,
            latency_ms=1000.0,
            model="gpt-4o-mini",
            seed=42,
            prompt_hash="hash",
            output_hash="out",
            cached=False,
        )
        params = ModelCallParams(
            model="gpt-4o-mini",
            prompt="Prompt",
            temperature=0.7,
            max_tokens=500,
        )
        
        events = []
        for i in range(3):
            event = tracker.record_model_call(
                function_id=f"gfn_{i}",
                model_call_result=result,
                params=params,
                execution_id=f"exec_{i}",
            )
            events.append(event)
        
        # Record batch
        batch_event = tracker.record_batch(
            batch_id="batch_123",
            model_name="gpt-4o-mini",
            call_events=events,
            function_ids=[f"gfn_{i}" for i in range(3)],
        )
        
        assert batch_event.batch_id == "batch_123"
        assert batch_event.batch_size == 3
        assert batch_event.total_cost_usd == pytest.approx(0.003)
        assert len(tracker.batch_events) == 1
    
    def test_tracker_with_empty_stats(self):
        """Test getting stats from empty tracker."""
        tracker = ModelCallTracker(tracker_id="empty")
        stats = tracker.get_stats()
        
        assert stats["total_calls"] == 0
        assert stats["total_cost_usd"] == 0.0
        assert stats["cache_hit_rate"] == 0.0
        assert stats["cache_hits"] == 0
    
    def test_invocation_index_increments(self):
        """Test that invocation index increments correctly."""
        tracker = ModelCallTracker(tracker_id="index_test")
        
        from modelado.core.model_call_client import ModelCallResult
        result = ModelCallResult(
            output="Output",
            cost_usd=0.001,
            latency_ms=1000.0,
            model="gpt-4o-mini",
            seed=None,
            prompt_hash="hash",
            output_hash="out",
            cached=False,
        )
        params = ModelCallParams(
            model="gpt-4o-mini",
            prompt="Prompt",
            temperature=0.7,
            max_tokens=500,
        )
        
        for i in range(5):
            event = tracker.record_model_call(
                function_id="gfn",
                model_call_result=result,
                params=params,
                execution_id="exec",
            )
            assert event.invocation_index == i
    
    def test_provider_detection(self):
        """Test automatic provider detection."""
        tracker = ModelCallTracker(tracker_id="provider_test")
        
        from modelado.core.model_call_client import ModelCallResult
        result = ModelCallResult(
            output="Output",
            cost_usd=0.0,
            latency_ms=100.0,
            model="model",
            seed=None,
            prompt_hash="hash",
            output_hash="out",
            cached=False,
        )
        
        # Test GPT detection
        params_gpt = ModelCallParams(
            model="gpt-4-turbo",
            prompt="Prompt",
            temperature=0.7,
            max_tokens=500,
        )
        event_gpt = tracker.record_model_call(
            function_id="gfn",
            model_call_result=result,
            params=params_gpt,
            execution_id="exec",
        )
        assert event_gpt.model_provider == "openai"
        
        # Test Claude detection
        params_claude = ModelCallParams(
            model="claude-opus",
            prompt="Prompt",
            temperature=0.7,
            max_tokens=500,
        )
        event_claude = tracker.record_model_call(
            function_id="gfn",
            model_call_result=result,
            params=params_claude,
            execution_id="exec",
        )
        assert event_claude.model_provider == "anthropic"


class TestModelCallProvenanceIntegration:
    """Integration tests for model call provenance."""
    
    def test_deterministic_replay_with_seed(self):
        """Test that calls with same seed can be replayed deterministically."""
        tracker = ModelCallTracker(tracker_id="replay_test")
        
        from modelado.core.model_call_client import ModelCallResult
        
        # First invocation
        result1 = ModelCallResult(
            output="Deterministic output",
            cost_usd=0.001,
            latency_ms=1000.0,
            model="gpt-4o-mini",
            seed=42,
            prompt_hash="consistent_hash",
            output_hash="output_hash_1",
            cached=False,
        )
        params1 = ModelCallParams(
            model="gpt-4o-mini",
            prompt="Same prompt",
            temperature=0.0,  # Deterministic
            max_tokens=500,
            seed=42,
        )
        event1 = tracker.record_model_call(
            function_id="gfn",
            model_call_result=result1,
            params=params1,
            execution_id="exec_1",
        )
        
        # Second invocation with same seed
        result2 = ModelCallResult(
            output="Deterministic output",
            cost_usd=0.0,  # Cached
            latency_ms=10.0,
            model="gpt-4o-mini",
            seed=42,
            prompt_hash="consistent_hash",
            output_hash="output_hash_1",  # Same hash
            cached=True,
        )
        params2 = ModelCallParams(
            model="gpt-4o-mini",
            prompt="Same prompt",
            temperature=0.0,
            max_tokens=500,
            seed=42,
        )
        event2 = tracker.record_model_call(
            function_id="gfn",
            model_call_result=result2,
            params=params2,
            execution_id="exec_2",
        )
        
        # Verify determinism
        assert event1.seed == event2.seed
        assert event1.prompt_hash == event2.prompt_hash
        assert event1.output_hash == event2.output_hash
        assert event2.was_cached is True
    
    def test_cost_aggregation_across_models(self):
        """Test cost aggregation when using multiple models."""
        tracker = ModelCallTracker(tracker_id="cost_test")
        
        from modelado.core.model_call_client import ModelCallResult
        
        # GPT-4o-mini call
        result_gpt = ModelCallResult(
            output="Output from GPT",
            cost_usd=0.001,
            latency_ms=1000.0,
            model="gpt-4o-mini",
            seed=None,
            prompt_hash="hash1",
            output_hash="out1",
            cached=False,
        )
        params_gpt = ModelCallParams(
            model="gpt-4o-mini",
            prompt="Prompt",
            temperature=0.7,
            max_tokens=500,
        )
        tracker.record_model_call(
            function_id="gfn",
            model_call_result=result_gpt,
            params=params_gpt,
            execution_id="exec",
        )
        
        # Claude call (higher cost)
        result_claude = ModelCallResult(
            output="Output from Claude",
            cost_usd=0.002,
            latency_ms=800.0,
            model="claude-opus",
            seed=None,
            prompt_hash="hash2",
            output_hash="out2",
            cached=False,
        )
        params_claude = ModelCallParams(
            model="claude-opus",
            prompt="Prompt",
            temperature=0.7,
            max_tokens=1000,
        )
        tracker.record_model_call(
            function_id="gfn",
            model_call_result=result_claude,
            params=params_claude,
            execution_id="exec",
        )
        
        stats = tracker.get_stats()
        
        # Total should be sum of both
        assert stats["total_cost_usd"] == pytest.approx(0.003)
        assert len(stats["models_used"]) == 2
