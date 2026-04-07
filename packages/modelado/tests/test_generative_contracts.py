"""Unit tests for generative contracts.

Tests cover:
1. Contract creation and validation
2. Serialization/deserialization
3. Content hashing and caching
4. Constraint enforcement
5. Factory methods
"""

import pytest
from datetime import datetime
from modelado.core.generative_contracts import (
    GenerativeCommand,
    GeneratedOperation,
    ExecutableFunction,
    ValidationResult,
    ValidationResults,
    ConstraintEnforcement,
    GenerationStrategy,
    ConstraintType,
    ValidationStatus,
)


class TestExecutableFunction:
    """Tests for ExecutableFunction contract."""

    def test_create_simple_function(self):
        """Test creating a basic executable function."""
        func = ExecutableFunction(
            name="test_func",
            language="python",
            code="def test(): return 42",
            signature={"inputs": {}, "outputs": {"result": "int"}},
            constraints_enforced=[ConstraintType.DETERMINISTIC],
            generation_strategy=GenerationStrategy.LLM_BASED,
            strategy_metadata={"model": "gpt-4o-mini"},
        )
        
        assert func.name == "test_func"
        assert func.language == "python"
        assert func.function_id  # Should be populated
        assert len(func.function_id) > 0

    def test_function_id_content_addressed(self):
        """Test that function_id is based on code content."""
        code1 = "def test(): return 42"
        code2 = "def test(): return 43"
        
        func1 = ExecutableFunction(
            name="f1", language="python", code=code1,
            signature={}, constraints_enforced=[],
            generation_strategy=GenerationStrategy.LLM_BASED,
            strategy_metadata={},
        )
        
        func2 = ExecutableFunction(
            name="f2", language="python", code=code2,
            signature={}, constraints_enforced=[],
            generation_strategy=GenerationStrategy.LLM_BASED,
            strategy_metadata={},
        )
        
        # Different code should produce different IDs
        assert func1.function_id != func2.function_id

    def test_same_code_same_id(self):
        """Test that same code always produces same function_id."""
        code = "def test(): return 42"
        
        func1 = ExecutableFunction(
            name="f1", language="python", code=code,
            signature={}, constraints_enforced=[],
            generation_strategy=GenerationStrategy.LLM_BASED,
            strategy_metadata={},
        )
        
        func2 = ExecutableFunction(
            name="f2", language="python", code=code,
            signature={}, constraints_enforced=[],
            generation_strategy=GenerationStrategy.LLM_BASED,
            strategy_metadata={},
        )
        
        # Same code should produce same ID
        assert func1.function_id == func2.function_id

    def test_function_serialization(self):
        """Test function can be serialized to dict."""
        func = ExecutableFunction(
            name="test", language="python", code="return 42",
            signature={"inputs": {}, "outputs": {}},
            constraints_enforced=[ConstraintType.DETERMINISTIC],
            generation_strategy=GenerationStrategy.LLM_BASED,
            strategy_metadata={},
        )
        
        d = func.to_dict()
        assert d["name"] == "test"
        assert d["language"] == "python"
        assert d["generation_strategy"] == "llm_based"
        assert "function_id" in d
        assert "generated_at" in d


class TestGenerativeCommand:
    """Tests for GenerativeCommand contract."""

    def test_create_command(self):
        """Test creating a basic generative command."""
        cmd = GenerativeCommand.create(
            user_instruction="Correlate revenue with market size",
            operation_type="economic_function",
            context={"project_id": "proj1", "artifact_id": "art1"},
        )
        
        assert cmd.command_id
        assert cmd.user_instruction == "Correlate revenue with market size"
        assert cmd.operation_type == "economic_function"
        assert ConstraintType.DETERMINISTIC in cmd.constraints

    def test_command_hash_deterministic(self):
        """Test that command hash is deterministic."""
        context = {"project_id": "proj1", "artifact_id": "art1"}
        
        cmd1 = GenerativeCommand.create(
            user_instruction="Correlate revenue with market size",
            operation_type="economic_function",
            context=context,
        )
        
        cmd2 = GenerativeCommand.create(
            user_instruction="Correlate revenue with market size",
            operation_type="economic_function",
            context=context,
        )
        
        # Same instruction and context should produce same hash
        assert cmd1.command_hash() == cmd2.command_hash()

    def test_command_hash_varies_with_instruction(self):
        """Test that command hash changes with instruction."""
        context = {"project_id": "proj1"}
        
        cmd1 = GenerativeCommand.create(
            user_instruction="Correlate revenue with market size",
            operation_type="economic_function",
            context=context,
        )
        
        cmd2 = GenerativeCommand.create(
            user_instruction="Calculate sensitivity analysis",
            operation_type="economic_function",
            context=context,
        )
        
        assert cmd1.command_hash() != cmd2.command_hash()

    def test_command_hash_varies_with_context(self):
        """Test that command hash changes with context."""
        instruction = "Correlate revenue with market size"
        
        cmd1 = GenerativeCommand.create(
            user_instruction=instruction,
            operation_type="economic_function",
            context={"project_id": "proj1"},
        )
        
        cmd2 = GenerativeCommand.create(
            user_instruction=instruction,
            operation_type="economic_function",
            context={"project_id": "proj2"},
        )
        
        assert cmd1.command_hash() != cmd2.command_hash()

    def test_command_with_custom_constraints(self):
        """Test creating command with custom constraints."""
        constraints = [
            ConstraintType.DETERMINISTIC,
            ConstraintType.BOUNDS_CHECK,
            ConstraintType.LOSSLESS_RECONSTRUCTION,
        ]
        
        cmd = GenerativeCommand.create(
            user_instruction="Custom operation",
            operation_type="economic_function",
            context={},
            constraints=constraints,
        )
        
        assert cmd.constraints == constraints

    def test_command_serialization(self):
        """Test command can be serialized to dict."""
        cmd = GenerativeCommand.create(
            user_instruction="Test instruction",
            operation_type="economic_function",
            context={"key": "value"},
        )
        
        d = cmd.to_dict()
        assert d["user_instruction"] == "Test instruction"
        assert d["operation_type"] == "economic_function"
        assert d["context"] == {"key": "value"}
        assert "command_id" in d
        assert "requested_at" in d


class TestValidationResult:
    """Tests for ValidationResult contract."""

    def test_create_passing_validation(self):
        """Test creating a passing validation result."""
        result = ValidationResult(
            constraint_type=ConstraintType.DETERMINISTIC,
            status=ValidationStatus.PASSED,
            message="Constraint satisfied",
        )
        
        assert result.constraint_type == ConstraintType.DETERMINISTIC
        assert result.status == ValidationStatus.PASSED
        assert result.is_success()

    def test_create_failing_validation(self):
        """Test creating a failing validation result."""
        result = ValidationResult(
            constraint_type=ConstraintType.BOUNDS_CHECK,
            status=ValidationStatus.FAILED,
            message="Value out of bounds",
            details={"min": 0, "max": 100, "actual": 150},
        )
        
        assert not result.is_success()
        assert result.details["actual"] == 150

    def test_validation_result_serialization(self):
        """Test validation result serialization."""
        result = ValidationResult(
            constraint_type=ConstraintType.DETERMINISTIC,
            status=ValidationStatus.PASSED,
            message="Test",
        )
        
        d = result.to_dict()
        assert d["constraint_type"] == "deterministic"
        assert d["status"] == "passed"
        assert "timestamp" in d


class TestValidationResults:
    """Tests for ValidationResults collection."""

    def test_add_passing_results(self):
        """Test adding passing results."""
        results = ValidationResults()
        
        r1 = ValidationResult(
            constraint_type=ConstraintType.DETERMINISTIC,
            status=ValidationStatus.PASSED,
            message="OK",
        )
        r2 = ValidationResult(
            constraint_type=ConstraintType.BOUNDS_CHECK,
            status=ValidationStatus.PASSED,
            message="OK",
        )
        
        results.add_result(r1)
        results.add_result(r2)
        
        assert results.passed_count == 2
        assert results.failed_count == 0
        assert results.is_valid()

    def test_add_failing_result(self):
        """Test that one failing result marks collection as invalid."""
        results = ValidationResults()
        
        r1 = ValidationResult(
            constraint_type=ConstraintType.DETERMINISTIC,
            status=ValidationStatus.PASSED,
            message="OK",
        )
        r2 = ValidationResult(
            constraint_type=ConstraintType.BOUNDS_CHECK,
            status=ValidationStatus.FAILED,
            message="FAIL",
        )
        
        results.add_result(r1)
        results.add_result(r2)
        
        assert results.passed_count == 1
        assert results.failed_count == 1
        assert not results.is_valid()

    def test_add_warning_result(self):
        """Test adding warning results."""
        results = ValidationResults()
        
        r1 = ValidationResult(
            constraint_type=ConstraintType.PERFORMANCE,
            status=ValidationStatus.WARNING,
            message="Slow",
        )
        
        results.add_result(r1)
        
        assert results.warning_count == 1
        assert results.overall_status == ValidationStatus.WARNING
        assert results.is_valid()  # Warning doesn't fail


class TestGeneratedOperation:
    """Tests for GeneratedOperation contract."""

    def test_create_operation(self):
        """Test creating a generated operation."""
        func = ExecutableFunction(
            name="test", language="python", code="return 42",
            signature={}, constraints_enforced=[],
            generation_strategy=GenerationStrategy.LLM_BASED,
            strategy_metadata={},
        )
        
        op = GeneratedOperation.create(
            command_id="cmd1",
            generated_function=func,
            semantic_confidence=0.95,
        )
        
        assert op.operation_id
        assert op.command_id == "cmd1"
        assert op.generated_function == func
        assert op.semantic_confidence == 0.95

    def test_operation_function_id_synced(self):
        """Test that operation's function_id stays in sync with generated_function."""
        func = ExecutableFunction(
            name="test", language="python", code="return 42",
            signature={}, constraints_enforced=[],
            generation_strategy=GenerationStrategy.LLM_BASED,
            strategy_metadata={},
        )
        
        op = GeneratedOperation.create(
            command_id="cmd1",
            generated_function=func,
        )
        
        assert op.function_id == func.function_id

    def test_operation_can_execute(self):
        """Test checking if operation is ready to execute."""
        func = ExecutableFunction(
            name="test", language="python", code="return 42",
            signature={}, constraints_enforced=[],
            generation_strategy=GenerationStrategy.LLM_BASED,
            strategy_metadata={},
        )
        
        op = GeneratedOperation.create(
            command_id="cmd1",
            generated_function=func,
        )
        
        # Should be valid and executable
        assert op.is_valid()
        assert op.can_execute()

    def test_operation_cannot_execute_if_invalid(self):
        """Test that operation with invalid validations cannot execute."""
        func = ExecutableFunction(
            name="test", language="python", code="return 42",
            signature={}, constraints_enforced=[],
            generation_strategy=GenerationStrategy.LLM_BASED,
            strategy_metadata={},
        )
        
        validation_results = ValidationResults()
        validation_results.add_result(ValidationResult(
            constraint_type=ConstraintType.DETERMINISTIC,
            status=ValidationStatus.FAILED,
            message="Not deterministic",
        ))
        
        op = GeneratedOperation.create(
            command_id="cmd1",
            generated_function=func,
            validation_results=validation_results,
        )
        
        assert not op.is_valid()
        assert not op.can_execute()

    def test_operation_serialization(self):
        """Test operation can be serialized."""
        func = ExecutableFunction(
            name="test", language="python", code="return 42",
            signature={}, constraints_enforced=[],
            generation_strategy=GenerationStrategy.LLM_BASED,
            strategy_metadata={},
        )
        
        op = GeneratedOperation.create(
            command_id="cmd1",
            generated_function=func,
            semantic_confidence=0.85,
        )
        
        d = op.to_dict()
        assert d["command_id"] == "cmd1"
        assert d["semantic_confidence"] == 0.85
        assert "operation_id" in d
        assert "generated_function" in d


class TestConstraintEnforcement:
    """Tests for ConstraintEnforcement record."""

    def test_create_enforcement_record(self):
        """Test creating constraint enforcement record."""
        validation = ValidationResult(
            constraint_type=ConstraintType.DETERMINISTIC,
            status=ValidationStatus.PASSED,
            message="OK",
        )
        
        enforcement = ConstraintEnforcement(
            constraint_type=ConstraintType.DETERMINISTIC,
            enforced=True,
            validation_result=validation,
        )
        
        assert enforcement.constraint_type == ConstraintType.DETERMINISTIC
        assert enforcement.enforced
        assert enforcement.validation_result == validation

    def test_enforcement_serialization(self):
        """Test enforcement record serialization."""
        enforcement = ConstraintEnforcement(
            constraint_type=ConstraintType.BOUNDS_CHECK,
            enforced=True,
        )
        
        d = enforcement.to_dict()
        assert d["constraint_type"] == "bounds_check"
        assert d["enforced"] is True


class TestIntegrationScenarios:
    """Integration tests combining multiple contracts."""

    def test_full_command_to_operation_flow(self):
        """Test complete flow from command creation to operation."""
        # Create command
        cmd = GenerativeCommand.create(
            user_instruction="Correlate revenue with market size using sigmoid",
            operation_type="economic_function",
            context={"project_id": "proj1", "artifact_id": "art1"},
            constraints=[ConstraintType.DETERMINISTIC, ConstraintType.LOSSLESS_RECONSTRUCTION],
        )
        
        # Create function (simulating generation)
        func = ExecutableFunction(
            name="correlate_revenue_market_size",
            language="python",
            code="""
def correlate_revenue_market_size(revenue, market_size):
    import math
    return revenue / (1 + math.exp(-market_size))
""",
            signature={
                "inputs": {"revenue": "float", "market_size": "float"},
                "outputs": {"result": "float"}
            },
            constraints_enforced=[ConstraintType.DETERMINISTIC],
            generation_strategy=GenerationStrategy.COMPOSABLE_BUILDING_BLOCKS,
            strategy_metadata={"building_blocks": ["sigmoid", "multiply"]},
        )
        
        # Create operation
        op = GeneratedOperation.create(
            command_id=cmd.command_id,
            generated_function=func,
            semantic_confidence=0.92,
            semantic_reasoning="Intent matches sigmoid correlation pattern",
        )
        
        # Verify flow
        assert op.command_id == cmd.command_id
        assert op.can_execute()
        assert op.semantic_confidence == 0.92

    def test_cached_operation_detection(self):
        """Test that same command can detect cached operation."""
        # First command
        cmd1 = GenerativeCommand.create(
            user_instruction="Correlate revenue with market size",
            operation_type="economic_function",
            context={"project_id": "proj1"},
        )
        
        # Second, identical command
        cmd2 = GenerativeCommand.create(
            user_instruction="Correlate revenue with market size",
            operation_type="economic_function",
            context={"project_id": "proj1"},
        )
        
        # Same context should produce same hash
        assert cmd1.command_hash() == cmd2.command_hash()

    def test_validation_enforcement_through_operation(self):
        """Test that constraints are properly recorded in operation."""
        func = ExecutableFunction(
            name="test", language="python", code="return 42",
            signature={}, constraints_enforced=[ConstraintType.DETERMINISTIC, ConstraintType.BOUNDS_CHECK],
            generation_strategy=GenerationStrategy.LLM_BASED,
            strategy_metadata={},
        )
        
        op = GeneratedOperation.create(
            command_id="cmd1",
            generated_function=func,
        )
        
        # Constraints should be recorded
        assert len(op.generated_function.constraints_enforced) == 2
        assert ConstraintType.DETERMINISTIC in op.generated_function.constraints_enforced


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
