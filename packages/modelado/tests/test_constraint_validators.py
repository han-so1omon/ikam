"""Unit tests for constraint validators.

Tests cover:
1. Each validator's validation logic
2. Validator registry and composition
3. Constraint checking for different function types
4. Error handling
5. Statistics collection
"""

import pytest

from modelado.core.generative_contracts import (
    ExecutableFunction,
    GenerationStrategy,
    ConstraintType,
    ValidationStatus,
)
from modelado.core.constraint_validators import (
    DeterminismValidator,
    BoundsCheckValidator,
    OutputRangeValidator,
    TypeSafetyValidator,
    LosslessReconstructionValidator,
    ConstraintValidatorRegistry,
    get_global_registry,
    validate_function,
)


# ============================================================================
# Test Helper Functions
# ============================================================================

def create_function(
    name: str = "test_func",
    code: str = "return 42",
    signature: dict = None,
) -> ExecutableFunction:
    """Helper to create test functions."""
    return ExecutableFunction(
        name=name,
        language="python",
        code=code,
        signature=signature or {},
        constraints_enforced=[],
        generation_strategy=GenerationStrategy.LLM_BASED,
        strategy_metadata={},
    )


# ============================================================================
# DeterminismValidator Tests
# ============================================================================

class TestDeterminismValidator:
    """Tests for DeterminismValidator."""
    
    def test_deterministic_function_passes(self):
        """Test that deterministic function passes."""
        validator = DeterminismValidator()
        func = create_function(
            code="""
def compute(x, y):
    return x * y + 42
"""
        )
        
        result = validator.validate(func)
        
        assert result.status == ValidationStatus.PASSED
        assert validator.passed_count == 1
        assert validator.failed_count == 0
    
    def test_random_function_fails(self):
        """Test that function with random() fails."""
        validator = DeterminismValidator()
        func = create_function(
            code="""
import random
def compute():
    return random.random()
"""
        )
        
        result = validator.validate(func)
        
        assert result.status == ValidationStatus.FAILED
        assert validator.failed_count == 1
        assert 'random' in str(result.details)
    
    def test_datetime_function_fails(self):
        """Test that function with datetime.now() fails."""
        validator = DeterminismValidator()
        func = create_function(
            code="""
from datetime import datetime
def compute():
    return datetime.now()
"""
        )
        
        result = validator.validate(func)
        
        assert result.status == ValidationStatus.FAILED
        assert validator.failed_count == 1
    
    def test_sleep_function_fails(self):
        """Test that function with time.sleep() fails."""
        validator = DeterminismValidator()
        func = create_function(
            code="""
import time
def compute():
    time.sleep(1)
    return 42
"""
        )
        
        result = validator.validate(func)
        
        assert result.status == ValidationStatus.FAILED
        assert validator.failed_count == 1
    
    def test_syntax_error_handled(self):
        """Test that syntax errors are handled gracefully."""
        validator = DeterminismValidator()
        func = create_function(code="this is not valid python ][")
        
        result = validator.validate(func)
        
        assert result.status == ValidationStatus.FAILED
        assert "Syntax error" in result.message
    
    def test_validator_stats(self):
        """Test validator statistics."""
        validator = DeterminismValidator()
        
        func1 = create_function(code="return 42")
        func2 = create_function(code="import random; return random.random()")
        
        validator.validate(func1)
        validator.validate(func2)
        
        stats = validator.get_stats()
        
        assert stats['validation_count'] == 2
        assert stats['passed_count'] == 1
        assert stats['failed_count'] == 1
        assert stats['pass_rate_percent'] == 50


# ============================================================================
# BoundsCheckValidator Tests
# ============================================================================

class TestBoundsCheckValidator:
    """Tests for BoundsCheckValidator."""
    
    def test_function_with_if_validation_passes(self):
        """Test that function with if validation passes."""
        validator = BoundsCheckValidator()
        func = create_function(
            code="""
def clamp(x, min_val, max_val):
    if x < min_val:
        return min_val
    if x > max_val:
        return max_val
    return x
"""
        )
        
        result = validator.validate(func)
        
        assert result.status == ValidationStatus.PASSED
    
    def test_function_with_assert_passes(self):
        """Test that function with assert passes."""
        validator = BoundsCheckValidator()
        func = create_function(
            code="""
def safe_divide(a, b):
    assert b != 0, "Cannot divide by zero"
    return a / b
"""
        )
        
        result = validator.validate(func)
        
        assert result.status == ValidationStatus.PASSED
    
    def test_function_without_validation_warns(self):
        """Test that function without validation warns."""
        validator = BoundsCheckValidator()
        func = create_function(code="return x * y")
        
        result = validator.validate(func)
        
        assert result.status == ValidationStatus.WARNING
        assert "may not include bounds checking" in result.message


# ============================================================================
# OutputRangeValidator Tests
# ============================================================================

class TestOutputRangeValidator:
    """Tests for OutputRangeValidator."""
    
    def test_function_with_output_signature_passes(self):
        """Test that function with output signature passes."""
        validator = OutputRangeValidator()
        func = create_function(
            signature={'inputs': {}, 'outputs': {'result': 'float'}}
        )
        
        result = validator.validate(func)
        
        assert result.status == ValidationStatus.PASSED
    
    def test_function_without_output_signature_warns(self):
        """Test that function without output signature warns."""
        validator = OutputRangeValidator()
        func = create_function(signature={})
        
        result = validator.validate(func)
        
        assert result.status == ValidationStatus.WARNING


# ============================================================================
# TypeSafetyValidator Tests
# ============================================================================

class TestTypeSafetyValidator:
    """Tests for TypeSafetyValidator."""
    
    def test_function_with_complete_signature_passes(self):
        """Test that function with complete type signature passes."""
        validator = TypeSafetyValidator()
        func = create_function(
            signature={
                'inputs': {'x': 'float', 'y': 'float'},
                'outputs': {'result': 'float'},
            }
        )
        
        result = validator.validate(func)
        
        assert result.status == ValidationStatus.PASSED
    
    def test_function_with_incomplete_signature_warns(self):
        """Test that function with incomplete signature warns."""
        validator = TypeSafetyValidator()
        func = create_function(signature={'inputs': {}})
        
        result = validator.validate(func)
        
        assert result.status == ValidationStatus.WARNING


# ============================================================================
# LosslessReconstructionValidator Tests
# ============================================================================

class TestLosslessReconstructionValidator:
    """Tests for LosslessReconstructionValidator."""
    
    def test_decompose_function_passes(self):
        """Test that decompose function is recognized."""
        validator = LosslessReconstructionValidator()
        func = create_function(name="decompose_artifact")
        
        result = validator.validate(func)
        
        assert result.status == ValidationStatus.PASSED
    
    def test_reconstruct_function_passes(self):
        """Test that reconstruct function is recognized."""
        validator = LosslessReconstructionValidator()
        func = create_function(name="reconstruct_from_fragments")
        
        result = validator.validate(func)
        
        assert result.status == ValidationStatus.PASSED
    
    def test_fragment_function_passes(self):
        """Test that fragment function is recognized."""
        validator = LosslessReconstructionValidator()
        func = create_function(name="fragment_document")
        
        result = validator.validate(func)
        
        assert result.status == ValidationStatus.PASSED
    
    def test_other_function_not_applicable(self):
        """Test that other functions are not applicable."""
        validator = LosslessReconstructionValidator()
        func = create_function(name="calculate_something")
        
        result = validator.validate(func)
        
        assert result.status == ValidationStatus.NOT_APPLICABLE


# ============================================================================
# ConstraintValidatorRegistry Tests
# ============================================================================

class TestConstraintValidatorRegistry:
    """Tests for ConstraintValidatorRegistry."""
    
    def test_registry_has_default_validators(self):
        """Test that registry is initialized with default validators."""
        registry = ConstraintValidatorRegistry()
        
        # Should have all standard validators
        assert registry.get_validator(ConstraintType.DETERMINISTIC) is not None
        assert registry.get_validator(ConstraintType.BOUNDS_CHECK) is not None
        assert registry.get_validator(ConstraintType.OUTPUT_RANGE_VALIDATION) is not None
        assert registry.get_validator(ConstraintType.TYPE_SAFETY) is not None
    
    def test_validate_all_constraints(self):
        """Test validating against all constraints."""
        registry = ConstraintValidatorRegistry()
        func = create_function(
            code="return x * y",
            signature={
                'inputs': {'x': 'float', 'y': 'float'},
                'outputs': {'result': 'float'},
            }
        )
        
        results = registry.validate_all(func)
        
        assert results.passed_count > 0
        assert len(results.results) > 0
    
    def test_validate_specific_constraints(self):
        """Test validating against specific constraints."""
        registry = ConstraintValidatorRegistry()
        func = create_function()
        
        results = registry.validate_specific(
            func,
            [ConstraintType.DETERMINISTIC, ConstraintType.TYPE_SAFETY],
        )
        
        assert len(results.results) == 2
        types = {r.constraint_type for r in results.results}
        assert ConstraintType.DETERMINISTIC in types
        assert ConstraintType.TYPE_SAFETY in types
    
    def test_registry_stats(self):
        """Test getting registry statistics."""
        registry = ConstraintValidatorRegistry()
        func = create_function()
        
        registry.validate_all(func)
        stats = registry.get_stats()
        
        assert ConstraintType.DETERMINISTIC.value in stats
        assert isinstance(stats[ConstraintType.DETERMINISTIC.value], dict)


# ============================================================================
# Global Registry Tests
# ============================================================================

class TestGlobalRegistry:
    """Tests for global registry functions."""
    
    def test_get_global_registry(self):
        """Test getting global registry."""
        registry1 = get_global_registry()
        registry2 = get_global_registry()
        
        assert registry1 is registry2
    
    def test_validate_function(self):
        """Test validate_function convenience function."""
        func = create_function(
            code="return x * y",
            signature={
                'inputs': {'x': 'float', 'y': 'float'},
                'outputs': {'result': 'float'},
            }
        )
        
        results = validate_function(func)
        
        assert results.passed_count > 0
        assert len(results.results) > 0


# ============================================================================
# Integration Tests
# ============================================================================

class TestValidationIntegration:
    """Integration tests for constraint validation."""
    
    def test_good_function_passes_multiple_constraints(self):
        """Test that well-formed function passes multiple constraints."""
        func = create_function(
            name="process_data",
            code="""
def process_data(values):
    if not values:
        return []
    result = [v * 2 for v in values]
    return result
""",
            signature={
                'inputs': {'values': 'list'},
                'outputs': {'result': 'list'},
            }
        )
        
        registry = ConstraintValidatorRegistry()
        results = registry.validate_all(func)
        
        # Should pass determinism and type safety
        determinism_results = [r for r in results.results if r.constraint_type == ConstraintType.DETERMINISTIC]
        assert any(r.status == ValidationStatus.PASSED for r in determinism_results)
    
    def test_bad_function_fails_multiple_constraints(self):
        """Test that poorly-formed function fails multiple constraints."""
        func = create_function(
            code="""
import random
def bad_function():
    return random.random()
""",
            signature={}  # No type signature
        )
        
        registry = ConstraintValidatorRegistry()
        results = registry.validate_all(func)
        
        # Should fail determinism
        assert not results.is_valid()
        
        # Should have at least one failure
        assert results.failed_count > 0


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
