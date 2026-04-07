"""Phase 1 integration tests for complete generative operations system.

PHASE 1 TASKS 1-4 INTEGRATION: End-to-end testing of all components.

Tests validate:
1. Complete flow: command → router → handler → cache → validation
2. Multi-handler dispatch with concurrent execution
3. Cache coherency across multiple requests
4. Constraint validation before execution
5. Error handling and recovery
"""

import pytest
import asyncio
from typing import Dict, Any, Optional

from modelado.core.generative_contracts import (
    GenerativeCommand,
    GeneratedOperation,
    ExecutableFunction,
    GenerationStrategy,
    ValidationStatus,
)
from modelado.core.generative_router import (
    GenerativeHandler,
    GenerativeCommandRouter,
    HandlerRegistry,
)
from modelado.core.constraint_validators import (
    ConstraintValidator,
    ConstraintValidatorRegistry,
    DeterminismValidator,
    BoundsCheckValidator,
    OutputRangeValidator,
)
from modelado.execution_cache import ExecutionCache


# ============================================================================
# Mock Handlers
# ============================================================================

class MockEconomicHandler(GenerativeHandler):
    """Mock handler for economic operations."""
    
    def __init__(self):
        super().__init__(
            name="MockEconomicHandler",
            operation_type="economic",
        )
        self.call_count = 0
    
    async def handle(self, command: GenerativeCommand) -> GeneratedOperation:
        """Generate mock economic operation."""
        self.call_count += 1
        
        # Simulate some processing
        await asyncio.sleep(0.01)
        
        func = ExecutableFunction(
            name="economic_model",
            language="python",
            code="def revenue(x): return x * 2.5",
            signature="(float) -> float",
        )
        
        operation = GeneratedOperation.create(
            command=command,
            generated_function=func,
        )
        
        return operation


class MockStoryHandler(GenerativeHandler):
    """Mock handler for story operations."""
    
    def __init__(self):
        super().__init__(
            name="MockStoryHandler",
            operation_type="story",
        )
        self.call_count = 0
    
    async def handle(self, command: GenerativeCommand) -> GeneratedOperation:
        """Generate mock story operation."""
        self.call_count += 1
        
        # Simulate some processing
        await asyncio.sleep(0.01)
        
        func = ExecutableFunction(
            name="story_generator",
            language="python",
            code="def generate_narrative(): return 'Once upon a time...'",
            signature="() -> str",
        )
        
        operation = GeneratedOperation.create(
            command=command,
            generated_function=func,
        )
        
        return operation


# ============================================================================
# Fixtures
# ============================================================================

@pytest.fixture
def handler_registry() -> HandlerRegistry:
    """Create registry with mock handlers."""
    registry = HandlerRegistry()
    registry.register(MockEconomicHandler(), priority=100)
    registry.register(MockStoryHandler(), priority=100)
    return registry


@pytest.fixture
def validator_registry() -> ConstraintValidatorRegistry:
    """Create validator registry."""
    registry = ConstraintValidatorRegistry()
    return registry


@pytest.fixture
async def router(handler_registry) -> GenerativeCommandRouter:
    """Create router with handlers and cache."""
    cache = ExecutionCache(memory_size=100)
    return GenerativeCommandRouter(
        handler_registry=handler_registry,
        execution_cache=cache,
    )


@pytest.fixture
def economic_command() -> GenerativeCommand:
    """Create economic operation command."""
    return GenerativeCommand.create(
        user_instruction="Build a revenue model with growth projections",
        operation_type="economic",
    )


@pytest.fixture
def story_command() -> GenerativeCommand:
    """Create story operation command."""
    return GenerativeCommand.create(
        user_instruction="Generate a narrative arc around market expansion",
        operation_type="story",
    )


# ============================================================================
# Phase 1 Integration Tests
# ============================================================================

class TestPhase1Integration:
    """Integration tests for complete Phase 1 system."""
    
    @pytest.mark.asyncio
    async def test_simple_command_routing(self, router, economic_command):
        """Test basic command routing."""
        operation = await router.route(economic_command)
        
        assert operation is not None
        assert operation.operation_id is not None
        assert operation.command_id == economic_command.command_id
        assert operation.generated_function.name == "economic_model"
    
    @pytest.mark.asyncio
    async def test_cache_hit_after_routing(self, router, economic_command):
        """Test cache stores result and returns on second request."""
        # First call: cache miss, generates operation
        operation1 = await router.route(economic_command)
        assert operation1.is_cached is False
        
        # Create new command with same content (same hash)
        command2 = GenerativeCommand.create(
            user_instruction=economic_command.user_instruction,
            operation_type=economic_command.operation_type,
        )
        
        # Second call: cache hit
        operation2 = await router.route(command2)
        assert operation2.is_cached is True
        assert operation2.operation_id == operation1.operation_id
    
    @pytest.mark.asyncio
    async def test_multiple_operation_types(self, router, economic_command, story_command):
        """Test routing different operation types."""
        econ_op = await router.route(economic_command)
        story_op = await router.route(story_command)
        
        assert econ_op.generated_function.name == "economic_model"
        assert story_op.generated_function.name == "story_generator"
    
    @pytest.mark.asyncio
    async def test_concurrent_routing(self, router):
        """Test concurrent command processing."""
        commands = [
            GenerativeCommand.create(
                user_instruction=f"Operation {i}",
                operation_type="economic" if i % 2 == 0 else "story",
            )
            for i in range(5)
        ]
        
        operations = await router.route_batch(commands, concurrent=True)
        
        assert len(operations) == 5
        for op in operations:
            assert op.operation_id is not None
    
    @pytest.mark.asyncio
    async def test_constraint_validation_integration(
        self,
        router,
        economic_command,
        validator_registry,
    ):
        """Test constraint validation during routing."""
        operation = await router.route(economic_command)
        
        # Validate the generated function
        results = validator_registry.validate_all(
            operation.generated_function.code,
        )
        
        # Should pass basic constraints
        assert results.overall_status == ValidationStatus.PASSED
    
    @pytest.mark.asyncio
    async def test_handler_not_found(self, handler_registry):
        """Test error when handler not found."""
        router = GenerativeCommandRouter(
            handler_registry=handler_registry,
            execution_cache=ExecutionCache(),
        )
        
        command = GenerativeCommand.create(
            user_instruction="Unknown operation",
            operation_type="unknown",  # No handler for this
        )
        
        with pytest.raises(ValueError, match="No handler found"):
            await router.route(command)
    
    @pytest.mark.asyncio
    async def test_cache_statistics_accumulation(self, router, economic_command):
        """Test cache statistics track multiple operations."""
        # First operation: cache miss
        await router.route(economic_command)
        
        # Second operation: cache hit
        command2 = GenerativeCommand.create(
            user_instruction=economic_command.user_instruction,
            operation_type=economic_command.operation_type,
        )
        await router.route(command2)
        
        # Third operation: cache miss (different command)
        command3 = GenerativeCommand.create(
            user_instruction="Different operation",
            operation_type=economic_command.operation_type,
        )
        await router.route(command3)
        
        stats = router.get_stats()
        assert stats['memory']['hits'] == 1
        assert stats['memory']['misses'] == 2
    
    @pytest.mark.asyncio
    async def test_handler_is_only_called_once_per_command(self, handler_registry):
        """Test handler is only called once due to caching."""
        econ_handler = handler_registry.get_handler("economic")
        initial_calls = econ_handler.call_count
        
        cache = ExecutionCache(memory_size=100)
        router = GenerativeCommandRouter(
            handler_registry=handler_registry,
            execution_cache=cache,
        )
        
        command = GenerativeCommand.create(
            user_instruction="Test operation",
            operation_type="economic",
        )
        
        # First call: handler is invoked
        await router.route(command)
        assert econ_handler.call_count == initial_calls + 1
        
        # Second call (same command): handler NOT invoked due to cache
        command2 = GenerativeCommand.create(
            user_instruction=command.user_instruction,
            operation_type=command.operation_type,
        )
        await router.route(command2)
        assert econ_handler.call_count == initial_calls + 1  # Not incremented!


# ============================================================================
# End-to-End Workflow Tests
# ============================================================================

class TestEndToEndWorkflow:
    """Complete end-to-end workflow tests."""
    
    @pytest.mark.asyncio
    async def test_full_pipeline_economic_operation(self, handler_registry):
        """Test complete pipeline for economic operation."""
        cache = ExecutionCache(memory_size=100)
        router = GenerativeCommandRouter(
            handler_registry=handler_registry,
            execution_cache=cache,
        )
        
        # Step 1: Create command
        command = GenerativeCommand.create(
            user_instruction="Generate revenue model with ARR projections",
            operation_type="economic",
        )
        
        # Step 2: Route and generate
        operation = await router.route(command)
        
        # Step 3: Validate result
        assert operation is not None
        assert operation.command_id == command.command_id
        assert operation.generated_function is not None
        assert operation.generated_function.code is not None
        
        # Step 4: Verify can execute
        assert operation.can_execute()
        
        # Step 5: Check cache populated
        cached_op = cache.get(command)
        assert cached_op is not None
        assert cached_op['operation_id'] == operation.operation_id
    
    @pytest.mark.asyncio
    async def test_full_pipeline_story_operation(self, handler_registry):
        """Test complete pipeline for story operation."""
        cache = ExecutionCache(memory_size=100)
        router = GenerativeCommandRouter(
            handler_registry=handler_registry,
            execution_cache=cache,
        )
        
        # Step 1: Create command
        command = GenerativeCommand.create(
            user_instruction="Create investor narrative focused on market TAM",
            operation_type="story",
        )
        
        # Step 2: Route and generate
        operation = await router.route(command)
        
        # Step 3: Verify structure
        assert operation.generated_function.name == "story_generator"
        assert "narrative" in operation.generated_function.code.lower()
        
        # Step 4: Verify execution ready
        assert operation.can_execute()
    
    @pytest.mark.asyncio
    async def test_mixed_workflow_multiple_operations(self, handler_registry):
        """Test workflow with multiple mixed operation types."""
        cache = ExecutionCache(memory_size=100)
        router = GenerativeCommandRouter(
            handler_registry=handler_registry,
            execution_cache=cache,
        )
        
        operations = []
        
        # Create mix of economic and story operations
        for i in range(3):
            cmd_econ = GenerativeCommand.create(
                user_instruction=f"Economic operation {i}",
                operation_type="economic",
            )
            op_econ = await router.route(cmd_econ)
            operations.append(("economic", op_econ))
            
            cmd_story = GenerativeCommand.create(
                user_instruction=f"Story operation {i}",
                operation_type="story",
            )
            op_story = await router.route(cmd_story)
            operations.append(("story", op_story))
        
        # Verify all operations generated
        assert len(operations) == 6
        
        # Verify types
        econ_ops = [op for t, op in operations if t == "economic"]
        story_ops = [op for t, op in operations if t == "story"]
        assert len(econ_ops) == 3
        assert len(story_ops) == 3
        
        # Verify all are ready to execute
        for _, op in operations:
            assert op.can_execute()
    
    @pytest.mark.asyncio
    async def test_cache_preserves_determinism(self, handler_registry):
        """Test that caching maintains deterministic behavior."""
        cache = ExecutionCache(memory_size=100)
        router = GenerativeCommandRouter(
            handler_registry=handler_registry,
            execution_cache=cache,
        )
        
        # Same instruction requested 3 times
        instruction = "Build quarterly forecast model"
        
        operations = []
        for i in range(3):
            cmd = GenerativeCommand.create(
                user_instruction=instruction,
                operation_type="economic",
            )
            op = await router.route(cmd)
            operations.append(op)
        
        # All should have same operation_id (from cache)
        assert operations[0].operation_id == operations[1].operation_id
        assert operations[1].operation_id == operations[2].operation_id
        
        # All should have same function code
        assert operations[0].generated_function.code == operations[1].generated_function.code
        assert operations[1].generated_function.code == operations[2].generated_function.code


# ============================================================================
# System Properties Tests
# ============================================================================

class TestSystemProperties:
    """Tests for system-level properties and guarantees."""
    
    @pytest.mark.asyncio
    async def test_idempotence(self, handler_registry):
        """Test that same command produces same result (idempotent)."""
        cache = ExecutionCache(memory_size=100)
        router = GenerativeCommandRouter(
            handler_registry=handler_registry,
            execution_cache=cache,
        )
        
        command = GenerativeCommand.create(
            user_instruction="Test idempotence",
            operation_type="economic",
        )
        
        # Call twice
        result1 = await router.route(command)
        
        command2 = GenerativeCommand.create(
            user_instruction="Test idempotence",
            operation_type="economic",
        )
        result2 = await router.route(command2)
        
        # Same result
        assert result1.operation_id == result2.operation_id
        assert result1.generated_function.code == result2.generated_function.code
    
    @pytest.mark.asyncio
    async def test_different_commands_different_results(self, handler_registry):
        """Test that different commands produce different results."""
        cache = ExecutionCache(memory_size=100)
        router = GenerativeCommandRouter(
            handler_registry=handler_registry,
            execution_cache=cache,
        )
        
        cmd1 = GenerativeCommand.create(
            user_instruction="Revenue model",
            operation_type="economic",
        )
        op1 = await router.route(cmd1)
        
        cmd2 = GenerativeCommand.create(
            user_instruction="Cost structure model",
            operation_type="economic",
        )
        op2 = await router.route(cmd2)
        
        # Different commands should have different hashes
        assert cmd1.command_hash() != cmd2.command_hash()
        # And produce different operations
        assert op1.operation_id != op2.operation_id
    
    @pytest.mark.asyncio
    async def test_cache_improves_throughput(self, handler_registry):
        """Test that cache significantly improves throughput."""
        cache = ExecutionCache(memory_size=100)
        router = GenerativeCommandRouter(
            handler_registry=handler_registry,
            execution_cache=cache,
        )
        
        command = GenerativeCommand.create(
            user_instruction="High-frequency operation",
            operation_type="economic",
        )
        
        # Route same command 10 times
        for i in range(10):
            cmd = GenerativeCommand.create(
                user_instruction=command.user_instruction,
                operation_type=command.operation_type,
            )
            await router.route(cmd)
        
        stats = router.get_stats()
        
        # Should have 9 cache hits out of 10 requests
        assert stats['memory']['hits'] == 9
        assert stats['memory']['misses'] == 1
        assert stats['memory']['hit_rate_percent'] == 90.0
