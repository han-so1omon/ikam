"""Unit tests for GenerativeCommandRouter.

Tests cover:
1. Handler registration and priority
2. Cache operations (hits, misses, LRU eviction)
3. Command routing to correct handler
4. Batch processing
5. Error handling
6. Statistics collection
"""

import pytest
import asyncio
from datetime import datetime

from modelado.core.generative_contracts import (
    GenerativeCommand,
    GeneratedOperation,
    ExecutableFunction,
    GenerationStrategy,
    ConstraintType,
)
from modelado.core.generative_router import (
    GenerativeHandler,
    GenerativeCommandRouter,
    ExecutionCache,
    HandlerRegistry,
)
from modelado.core.deliberation_sink import DeliberationContext, DeliberationEnvelopePayload


# ============================================================================
# Test Handlers
# ============================================================================

class MockEconomicHandler(GenerativeHandler):
    """Mock handler for economic operations."""
    
    def __init__(self):
        super().__init__("mock_economic", "economic_function")
        self.generation_count = 0
    
    async def handle(self, command: GenerativeCommand) -> GeneratedOperation:
        """Generate a mock operation."""
        self.generation_count += 1
        
        func = ExecutableFunction(
            name=f"economic_func_{self.generation_count}",
            language="python",
            code="def economic_func(): return 42",
            signature={},
            constraints_enforced=[ConstraintType.DETERMINISTIC],
            generation_strategy=GenerationStrategy.LLM_BASED,
            strategy_metadata={"model": "gpt-4o-mini"},
        )
        
        return GeneratedOperation.create(
            command_id=command.command_id,
            generated_function=func,
            semantic_confidence=0.95,
        )


class MockStoryHandler(GenerativeHandler):
    """Mock handler for story operations."""
    
    def __init__(self):
        super().__init__("mock_story", "story_operation")
        self.generation_count = 0
    
    async def handle(self, command: GenerativeCommand) -> GeneratedOperation:
        """Generate a mock story operation."""
        self.generation_count += 1
        
        func = ExecutableFunction(
            name=f"story_func_{self.generation_count}",
            language="python",
            code="def story_func(): return 'narrative'",
            signature={},
            constraints_enforced=[],
            generation_strategy=GenerationStrategy.TEMPLATE_INJECTION,
            strategy_metadata={},
        )
        
        return GeneratedOperation.create(
            command_id=command.command_id,
            generated_function=func,
        )


class FailingHandler(GenerativeHandler):
    """Handler that always fails."""
    
    def __init__(self):
        super().__init__("failing", "failing_operation")
    
    async def handle(self, command: GenerativeCommand) -> GeneratedOperation:
        """Raise an error."""
        raise RuntimeError("Handler failure")


# ============================================================================
# ExecutionCache Tests
# ============================================================================

class TestExecutionCache:
    """Tests for ExecutionCache."""
    
    def test_cache_miss_on_empty_cache(self):
        """Test cache miss when cache is empty."""
        cache = ExecutionCache()
        cmd = GenerativeCommand.create(
            user_instruction="Test",
            operation_type="economic_function",
            context={},
        )
        
        result = cache.get(cmd)
        
        assert result is None
        assert cache.misses == 1
        assert cache.hits == 0
    
    def test_cache_hit_after_put(self):
        """Test cache hit after putting an operation."""
        cache = ExecutionCache()
        
        cmd = GenerativeCommand.create(
            user_instruction="Test",
            operation_type="economic_function",
            context={},
        )
        
        func = ExecutableFunction(
            name="test", language="python", code="return 42",
            signature={}, constraints_enforced=[],
            generation_strategy=GenerationStrategy.LLM_BASED,
            strategy_metadata={},
        )
        
        op = GeneratedOperation.create(command_id=cmd.command_id, generated_function=func)
        cache.put(cmd, op)
        
        # Now should hit
        result = cache.get(cmd)
        assert result is not None
        assert result.operation_id == op.operation_id
        assert cache.hits == 1
        assert cache.misses == 0
    
    def test_cache_hit_rate_calculation(self):
        """Test hit rate calculation."""
        cache = ExecutionCache()
        
        # 3 misses
        cache.misses = 3
        cache.hits = 0
        
        stats = cache.get_stats()
        assert stats['hit_rate_percent'] == 0
        assert stats['total_requests'] == 3
        
        # 1 hit, 3 misses = 25%
        cache.hits = 1
        stats = cache.get_stats()
        assert stats['hit_rate_percent'] == 25
    
    def test_cache_lru_eviction(self):
        """Test LRU eviction when cache is full."""
        cache = ExecutionCache(max_size=2)
        
        # Add first entry
        cmd1 = GenerativeCommand.create(
            user_instruction="First",
            operation_type="economic_function",
            context={},
        )
        func1 = ExecutableFunction(
            name="f1", language="python", code="return 1",
            signature={}, constraints_enforced=[],
            generation_strategy=GenerationStrategy.LLM_BASED,
            strategy_metadata={},
        )
        op1 = GeneratedOperation.create(command_id=cmd1.command_id, generated_function=func1)
        cache.put(cmd1, op1)
        
        # Add second entry
        cmd2 = GenerativeCommand.create(
            user_instruction="Second",
            operation_type="economic_function",
            context={},
        )
        func2 = ExecutableFunction(
            name="f2", language="python", code="return 2",
            signature={}, constraints_enforced=[],
            generation_strategy=GenerationStrategy.LLM_BASED,
            strategy_metadata={},
        )
        op2 = GeneratedOperation.create(command_id=cmd2.command_id, generated_function=func2)
        cache.put(cmd2, op2)
        
        # Cache is full (size 2)
        assert len(cache._cache) == 2
        
        # Add third entry - should evict first (LRU)
        cmd3 = GenerativeCommand.create(
            user_instruction="Third",
            operation_type="economic_function",
            context={},
        )
        func3 = ExecutableFunction(
            name="f3", language="python", code="return 3",
            signature={}, constraints_enforced=[],
            generation_strategy=GenerationStrategy.LLM_BASED,
            strategy_metadata={},
        )
        op3 = GeneratedOperation.create(command_id=cmd3.command_id, generated_function=func3)
        cache.put(cmd3, op3)
        
        # First entry should be evicted
        assert len(cache._cache) == 2
        assert cache.get(cmd1) is None
        assert cache.get(cmd2) is not None
        assert cache.get(cmd3) is not None
    
    def test_cache_clear(self):
        """Test clearing cache."""
        cache = ExecutionCache()
        
        cmd = GenerativeCommand.create(
            user_instruction="Test",
            operation_type="economic_function",
            context={},
        )
        
        func = ExecutableFunction(
            name="test", language="python", code="return 42",
            signature={}, constraints_enforced=[],
            generation_strategy=GenerationStrategy.LLM_BASED,
            strategy_metadata={},
        )
        
        op = GeneratedOperation.create(command_id=cmd.command_id, generated_function=func)
        cache.put(cmd, op)
        
        assert len(cache._cache) == 1
        
        cache.clear()
        
        assert len(cache._cache) == 0
        assert cache.hits == 0
        assert cache.misses == 0


# ============================================================================
# HandlerRegistry Tests
# ============================================================================

class TestHandlerRegistry:
    """Tests for HandlerRegistry."""
    
    def test_register_handler(self):
        """Test registering a handler."""
        registry = HandlerRegistry()
        handler = MockEconomicHandler()
        
        registry.register(handler)
        
        assert registry.get_handler("economic_function") == handler
    
    def test_handler_priority(self):
        """Test handler priority ordering."""
        registry = HandlerRegistry()
        
        h1 = MockEconomicHandler()
        h2 = MockEconomicHandler()
        h3 = MockEconomicHandler()
        
        registry.register(h1, priority=100)
        registry.register(h2, priority=50)  # Higher priority (lower number)
        registry.register(h3, priority=150)
        
        # Should get h2 (priority 50)
        assert registry.get_handler("economic_function") == h2
        
        # Get all handlers should be in priority order
        all_handlers = registry.get_all_handlers("economic_function")
        assert all_handlers[0] == h2  # priority 50
        assert all_handlers[1] == h1  # priority 100
        assert all_handlers[2] == h3  # priority 150
    
    def test_list_handlers(self):
        """Test listing registered handlers."""
        registry = HandlerRegistry()
        
        h1 = MockEconomicHandler()
        h2 = MockStoryHandler()
        
        registry.register(h1)
        registry.register(h2)
        
        listing = registry.list_handlers()
        
        assert "economic_function" in listing
        assert "story_operation" in listing
        assert h1.name in listing["economic_function"]
        assert h2.name in listing["story_operation"]


# ============================================================================
# GenerativeCommandRouter Tests
# ============================================================================

class TestGenerativeCommandRouter:
    """Tests for GenerativeCommandRouter."""
    
    @pytest.mark.asyncio
    async def test_route_simple_command(self):
        """Test routing a simple command."""
        router = GenerativeCommandRouter()
        handler = MockEconomicHandler()
        router.register_handler(handler)
        
        cmd = GenerativeCommand.create(
            user_instruction="Test economic operation",
            operation_type="economic_function",
            context={},
        )
        
        op = await router.route(cmd)
        
        assert op is not None
        assert op.command_id == cmd.command_id
        assert op.can_execute()
        assert handler.generation_count == 1
    
    @pytest.mark.asyncio
    async def test_route_with_cache_hit(self):
        """Test that second identical command hits cache."""
        router = GenerativeCommandRouter()
        handler = MockEconomicHandler()
        router.register_handler(handler)
        
        cmd1 = GenerativeCommand.create(
            user_instruction="Test",
            operation_type="economic_function",
            context={"key": "value"},
        )
        
        # First route - should generate
        op1 = await router.route(cmd1)
        assert handler.generation_count == 1
        assert not op1.is_cached
        
        # Create identical command
        cmd2 = GenerativeCommand.create(
            user_instruction="Test",
            operation_type="economic_function",
            context={"key": "value"},
        )
        
        # Second route - should hit cache
        op2 = await router.route(cmd2)
        assert handler.generation_count == 1  # Should not increment
        
        # Cached operation should be identical (same operation_id)
        assert op1.operation_id == op2.operation_id  # Same cached operation
        assert op2.is_cached is False  # is_cached is set at generation, not on retrieval
        assert cmd1.command_hash() == cmd2.command_hash()
    
    @pytest.mark.asyncio
    async def test_route_different_commands_no_cache_hit(self):
        """Test that different commands don't hit cache."""
        router = GenerativeCommandRouter()
        handler = MockEconomicHandler()
        router.register_handler(handler)
        
        cmd1 = await router.route(GenerativeCommand.create(
            user_instruction="First",
            operation_type="economic_function",
            context={},
        ))
        
        cmd2 = await router.route(GenerativeCommand.create(
            user_instruction="Second",
            operation_type="economic_function",
            context={},
        ))
        
        # Both should have generated (no cache hit)
        assert handler.generation_count == 2
    
    @pytest.mark.asyncio
    async def test_route_multiple_operation_types(self):
        """Test routing different operation types."""
        router = GenerativeCommandRouter()
        router.register_handler(MockEconomicHandler())
        router.register_handler(MockStoryHandler())
        
        cmd_econ = GenerativeCommand.create(
            user_instruction="Economic",
            operation_type="economic_function",
            context={},
        )
        
        cmd_story = GenerativeCommand.create(
            user_instruction="Story",
            operation_type="story_operation",
            context={},
        )
        
        op_econ = await router.route(cmd_econ)
        op_story = await router.route(cmd_story)
        
        assert op_econ is not None
        assert op_story is not None
        assert "economic" in op_econ.generated_function.name
        assert "story" in op_story.generated_function.name

    @pytest.mark.asyncio
    async def test_route_emits_deliberation_summary(self):
        """Ensure routing emits a safe deliberation summary via sink."""

        class RecordingSink:
            def __init__(self):
                self.events = []

            def emit(
                self,
                *,
                envelope: DeliberationEnvelopePayload,
                context: DeliberationContext,
            ) -> None:
                self.events.append((envelope, context))

        sink = RecordingSink()
        router = GenerativeCommandRouter(deliberation_sink=sink)
        handler = MockEconomicHandler()
        router.register_handler(handler)

        cmd = GenerativeCommand.create(
            user_instruction="Test routing summary",
            operation_type="economic_function",
            context={"project_id": "proj-1", "session_id": "sess-1"},
        )

        await router.route(cmd)

        assert len(sink.events) == 1
        envelope, ctx = sink.events[0]
        assert envelope.phase == "decide"
        assert envelope.status == "complete"
        assert envelope.summary == f"Routed {cmd.command_id} to mock_economic"
        assert cmd.user_instruction not in envelope.summary
        assert ctx.project_id == "proj-1"
        assert ctx.session_id == "sess-1"
        assert ctx.run_id == cmd.command_id
    
    @pytest.mark.asyncio
    async def test_route_no_handler_found(self):
        """Test error when no handler found."""
        router = GenerativeCommandRouter()
        
        cmd = GenerativeCommand.create(
            user_instruction="Test",
            operation_type="nonexistent_operation",
            context={},
        )
        
        with pytest.raises(ValueError, match="No handler registered"):
            await router.route(cmd)
    
    @pytest.mark.asyncio
    async def test_route_handler_failure(self):
        """Test error handling when handler fails."""
        router = GenerativeCommandRouter()
        router.register_handler(FailingHandler())
        
        cmd = GenerativeCommand.create(
            user_instruction="Test",
            operation_type="failing_operation",
            context={},
        )
        
        with pytest.raises(RuntimeError, match="Handler failure"):
            await router.route(cmd)
        
        assert router.total_errors == 1
    
    @pytest.mark.asyncio
    async def test_route_batch(self):
        """Test batch routing."""
        router = GenerativeCommandRouter()
        handler = MockEconomicHandler()
        router.register_handler(handler)
        
        commands = [
            GenerativeCommand.create(
                user_instruction=f"Command {i}",
                operation_type="economic_function",
                context={},
            )
            for i in range(3)
        ]
        
        operations = await router.route_batch(commands, concurrent=True)
        
        assert len(operations) == 3
        assert handler.generation_count == 3
    
    @pytest.mark.asyncio
    async def test_router_statistics(self):
        """Test router statistics collection."""
        router = GenerativeCommandRouter()
        handler = MockEconomicHandler()
        router.register_handler(handler)
        
        cmd1 = GenerativeCommand.create(
            user_instruction="Test 1",
            operation_type="economic_function",
            context={},
        )
        
        cmd2 = GenerativeCommand.create(
            user_instruction="Test 1",  # Same
            operation_type="economic_function",
            context={},
        )
        
        await router.route(cmd1)
        await router.route(cmd2)  # Cache hit
        
        stats = router.get_stats()
        
        assert stats['total_commands_processed'] == 2
        assert stats['total_errors'] == 0
        assert stats['cache_stats']['hits'] == 1
        assert stats['cache_stats']['misses'] == 1
    
    def test_list_handlers(self):
        """Test listing registered handlers."""
        router = GenerativeCommandRouter()
        router.register_handler(MockEconomicHandler())
        router.register_handler(MockStoryHandler())
        
        listing = router.list_handlers()
        
        assert "economic_function" in listing
        assert "story_operation" in listing


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
