"""
Phase 9.7 Task 7.1 Tests: Model Call Client & Batch Queue

Tests validate:
1. Model call client with deterministic seeding and caching
2. Cost/latency tracking and provenance recording
3. Batch queue with stable FIFO ordering
4. Idempotent writes (same prompt + seed = reuse item)
5. Backpressure enforcement
"""

import pytest
from datetime import datetime

from modelado.core.model_call_client import (
    ModelCallClient,
    ModelCallParams,
    ModelCallCache,
    ModelName,
)
from modelado.core.model_call_batch_queue import (
    ModelCallBatchQueue,
    BatchItem,
    Batch,
)


class TestModelCallCache:
    """Unit tests for ModelCallCache."""

    def test_cache_creation(self):
        """Test cache instantiation."""
        cache = ModelCallCache(max_entries=500)
        assert cache._max_entries == 500
        assert len(cache._cache) == 0

    def test_cache_put_and_get(self):
        """Test basic put/get operations."""
        cache = ModelCallCache()
        key = "test_key"
        value = "test_output"

        cache.put(key, value)
        assert cache.get(key) == value

    def test_cache_miss(self):
        """Test cache miss returns None."""
        cache = ModelCallCache()
        assert cache.get("nonexistent") is None

    def test_cache_stats(self):
        """Test cache statistics."""
        cache = ModelCallCache()
        
        # Put and hit
        cache.put("key1", "value1")
        assert cache.get("key1") is not None
        
        # Miss
        assert cache.get("key2") is None
        
        stats = cache.stats()
        assert stats["hits"] == 1
        assert stats["misses"] == 1
        assert stats["hit_rate"] == 0.5

    def test_cache_eviction(self):
        """Test cache eviction when full."""
        cache = ModelCallCache(max_entries=3)
        
        cache.put("key1", "value1")
        cache.put("key2", "value2")
        cache.put("key3", "value3")
        
        # Cache full, adding new key should evict first
        cache.put("key4", "value4")
        
        # key1 should be evicted (FIFO)
        assert cache.get("key1") is None
        assert cache.get("key4") == "value4"


@pytest.mark.asyncio
class TestModelCallClient:
    """Tests for ModelCallClient."""

    async def test_client_creation(self):
        """Test client instantiation."""
        client = ModelCallClient()
        assert client.client_id
        assert client.cache is not None
        assert len(client.cost_profiles) > 0

    async def test_model_validation(self):
        """Test model allowlist validation."""
        client = ModelCallClient()
        
        # Valid model
        valid_params = ModelCallParams(
            model=ModelName.GPT_4O_MINI,
            prompt="Test prompt",
        )
        is_valid, msg = client.validate_params(valid_params)
        assert is_valid

    async def test_temperature_validation(self):
        """Test temperature parameter validation."""
        client = ModelCallClient()
        
        # Invalid temperature (too high)
        invalid_params = ModelCallParams(
            model=ModelName.GPT_4O_MINI,
            prompt="Test",
            temperature=2.5,
        )
        is_valid, msg = client.validate_params(invalid_params)
        assert not is_valid
        assert "Temperature" in msg

    async def test_max_tokens_validation(self):
        """Test max_tokens parameter validation."""
        client = ModelCallClient()
        
        # Invalid max_tokens
        invalid_params = ModelCallParams(
            model=ModelName.GPT_4O_MINI,
            prompt="Test",
            max_tokens=5000,
        )
        is_valid, msg = client.validate_params(invalid_params)
        assert not is_valid
        assert "max_tokens" in msg

    async def test_model_call_with_deterministic_seed(self):
        """Test model call with deterministic seeding."""
        client = ModelCallClient()
        
        params = ModelCallParams(
            model=ModelName.GPT_4O_MINI,
            prompt="Test prompt",
            seed=12345,
        )
        
        result = await client.call(params)
        
        assert result.model == ModelName.GPT_4O_MINI
        assert result.seed == 12345
        assert result.cost_usd > 0
        assert result.latency_ms > 0
        assert not result.cached

    async def test_cache_hit_on_second_call(self):
        """Test cache hit on repeated calls."""
        client = ModelCallClient()
        
        params = ModelCallParams(
            model=ModelName.CLAUDE_HAIKU,
            prompt="Repeated prompt",
            seed=42,
        )
        
        # First call (cache miss)
        result1 = await client.call(params, use_cache=True)
        assert not result1.cached
        
        # Second call (cache hit)
        result2 = await client.call(params, use_cache=True)
        assert result2.cached
        assert result2.cost_usd == 0.0  # No cost for cache hit
        assert result2.output == result1.output

    async def test_cache_disabled(self):
        """Test disabling cache."""
        client = ModelCallClient()
        
        params = ModelCallParams(
            model=ModelName.GPT_4O_MINI,
            prompt="Test",
            seed=99,
        )
        
        result1 = await client.call(params, use_cache=False)
        result2 = await client.call(params, use_cache=False)
        
        assert not result1.cached
        assert not result2.cached

    async def test_call_history(self):
        """Test call history tracking."""
        client = ModelCallClient()
        
        params = ModelCallParams(
            model=ModelName.GPT_4O_MINI,
            prompt="Test",
        )
        
        await client.call(params)
        await client.call(params)
        
        history = client.get_call_history()
        assert len(history) == 2

    async def test_cache_stats(self):
        """Test cache statistics through client."""
        client = ModelCallClient()
        
        params = ModelCallParams(
            model=ModelName.CLAUDE_HAIKU,
            prompt="Cache stats test",
            seed=111,
        )
        
        await client.call(params)  # Cache miss
        await client.call(params)  # Cache hit
        
        stats = client.get_cache_stats()
        assert stats["hits"] == 1
        assert stats["misses"] == 1


@pytest.mark.asyncio
class TestModelCallBatchQueue:
    """Tests for ModelCallBatchQueue."""

    async def test_queue_creation(self):
        """Test batch queue instantiation."""
        queue = ModelCallBatchQueue()
        assert queue.queue_id
        assert queue.max_batch_size == 32
        assert len(queue._batches) == 0

    async def test_enqueue_creates_batch(self):
        """Test enqueuing creates a batch."""
        queue = ModelCallBatchQueue()
        
        params = ModelCallParams(
            model=ModelName.GPT_4O_MINI,
            prompt="Test prompt",
            seed=1,
        )
        
        item = await queue.enqueue(params, cost_usd=0.01)
        
        assert item.item_id
        assert item.queue_position == 0
        assert item.item_index == 0
        assert item.batch_id

    async def test_stable_fifo_ordering(self):
        """Test stable FIFO ordering with deterministic positions."""
        queue = ModelCallBatchQueue()
        
        # Enqueue multiple items
        items = []
        for i in range(5):
            params = ModelCallParams(
                model=ModelName.GPT_4O_MINI,
                prompt=f"Prompt {i}",
                seed=i,
            )
            item = await queue.enqueue(params)
            items.append(item)
        
        # Check queue positions are stable
        for i, item in enumerate(items):
            assert item.queue_position == i

    async def test_idempotent_enqueue(self):
        """Test idempotent enqueue (same params = reuse item)."""
        queue = ModelCallBatchQueue()
        
        params = ModelCallParams(
            model=ModelName.CLAUDE_HAIKU,
            prompt="Idempotent test",
            seed=42,
        )
        
        # First enqueue
        item1 = await queue.enqueue(params)
        
        # Second enqueue (same params)
        item2 = await queue.enqueue(params)
        
        # Should be the same item
        assert item1.item_id == item2.item_id

    async def test_batch_grouping_by_param_hash(self):
        """Test that items with same params group in same batch."""
        queue = ModelCallBatchQueue()
        
        params1 = ModelCallParams(
            model=ModelName.GPT_4O_MINI,
            prompt="Test",
            temperature=0.7,
            seed=1,
        )
        
        params2 = ModelCallParams(
            model=ModelName.GPT_4O_MINI,
            prompt="Test",  # Same prompt
            temperature=0.7,  # Same temp
            seed=2,  # Different seed
        )
        
        item1 = await queue.enqueue(params1)
        item2 = await queue.enqueue(params2)
        
        # Both should be in same batch (same prompt hash, same hyperparams)
        assert item1.batch_id == item2.batch_id

    async def test_separate_batches_for_different_params(self):
        """Test different params create separate batches."""
        queue = ModelCallBatchQueue()
        
        params1 = ModelCallParams(
            model=ModelName.GPT_4O_MINI,
            prompt="Prompt 1",
        )
        
        params2 = ModelCallParams(
            model=ModelName.CLAUDE_HAIKU,  # Different model
            prompt="Prompt 1",
        )
        
        item1 = await queue.enqueue(params1)
        item2 = await queue.enqueue(params2)
        
        # Different batches
        assert item1.batch_id != item2.batch_id

    async def test_backpressure_enforcement(self):
        """Test backpressure cap enforcement."""
        queue = ModelCallBatchQueue(backpressure_threshold=5)
        
        # Fill queue to threshold
        for i in range(5):
            params = ModelCallParams(
                model=ModelName.GPT_4O_MINI,
                prompt=f"Prompt {i}",
                seed=i,
            )
            await queue.enqueue(params)
        
        # Next enqueue should fail
        params = ModelCallParams(
            model=ModelName.GPT_4O_MINI,
            prompt="Over limit",
        )
        
        with pytest.raises(ValueError, match="backpressure"):
            await queue.enqueue(params)

    async def test_batch_submission(self):
        """Test batch submission."""
        queue = ModelCallBatchQueue()
        
        params = ModelCallParams(
            model=ModelName.GPT_4O_MINI,
            prompt="Submit test",
        )
        
        item = await queue.enqueue(params)
        param_hash = item.param_hash
        
        batch = await queue.submit_batch(param_hash)
        
        assert batch.submitted_at is not None
        assert batch.batch_id in queue._submitted_batches

    async def test_queue_statistics(self):
        """Test queue statistics."""
        queue = ModelCallBatchQueue()
        
        # Different seeds to avoid idempotent reuse
        params1 = ModelCallParams(
            model=ModelName.GPT_4O_MINI,
            prompt="Stats test",
            seed=1,
        )
        params2 = ModelCallParams(
            model=ModelName.GPT_4O_MINI,
            prompt="Stats test",
            seed=2,
        )
        
        await queue.enqueue(params1, cost_usd=0.05)
        await queue.enqueue(params2, cost_usd=0.05)
        
        stats = queue.get_stats()
        
        assert stats["total_batches"] == 1
        assert stats["total_items"] == 2
        assert stats["total_cost_usd"] == 0.10

    async def test_get_pending_batches(self):
        """Test retrieving pending (unsubmitted) batches."""
        queue = ModelCallBatchQueue()
        
        params1 = ModelCallParams(
            model=ModelName.GPT_4O_MINI,
            prompt="Prompt 1",
        )
        
        params2 = ModelCallParams(
            model=ModelName.CLAUDE_HAIKU,
            prompt="Prompt 2",
        )
        
        item1 = await queue.enqueue(params1)
        item2 = await queue.enqueue(params2)
        
        # Both pending
        pending = queue.get_pending_batches()
        assert len(pending) == 2
        
        # Submit one
        await queue.submit_batch(item1.param_hash)
        
        pending = queue.get_pending_batches()
        assert len(pending) == 1


@pytest.mark.asyncio
class TestIntegration:
    """Integration tests for Model Call Client + Batch Queue."""

    async def test_client_and_queue_together(self):
        """Test client and queue working together."""
        client = ModelCallClient()
        queue = ModelCallBatchQueue()
        
        params = ModelCallParams(
            model=ModelName.GPT_4O_MINI,
            prompt="Integration test",
            seed=999,
        )
        
        # Enqueue in batch queue
        item = await queue.enqueue(params)
        
        # Call client
        result = await client.call(params)
        
        # Verify consistency
        assert item.params.model == result.model
        assert item.seed_hash is not None
        assert result.cost_usd > 0

    async def test_deterministic_replay_with_seed(self):
        """Test deterministic replay using seed + batch queue."""
        queue = ModelCallBatchQueue()
        
        # Same prompt, same seed = deterministic
        params = ModelCallParams(
            model=ModelName.CLAUDE_HAIKU,
            prompt="Deterministic test",
            seed=777,
        )
        
        item1 = await queue.enqueue(params)
        item2 = await queue.enqueue(params)
        
        # Should be same item (idempotent)
        assert item1.item_id == item2.item_id
        assert item1.seed_hash == item2.seed_hash


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
