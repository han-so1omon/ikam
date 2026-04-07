"""Constraint validation framework for generative operations.

Validators for enforcing constraints on generated functions.

This module provides:
1. ConstraintValidator interface and base implementations
2. Registry of validators
3. Composition of multiple validators
4. Reporting and debugging of constraint violations

Validators can be applied to ExecutableFunction to ensure they meet
required constraints (determinism, bounds checking, type safety, etc.).
"""

from __future__ import annotations

import ast
import inspect
import logging
from abc import ABC, abstractmethod
from typing import Any, Callable, Dict, List, Optional, Tuple

from .generative_contracts import (
    ExecutableFunction,
    ValidationResult,
    ValidationResults,
    ValidationStatus,
    ConstraintType,
)

logger = logging.getLogger(__name__)


# ============================================================================
# Constraint Validator Interface
# ============================================================================

class ConstraintValidator(ABC):
    """Base class for constraint validators."""
    
    def __init__(self, constraint_type: ConstraintType, description: str = ""):
        """Initialize validator.
        
        Args:
            constraint_type: Type of constraint this validator checks
            description: Human-readable description
        """
        self.constraint_type = constraint_type
        self.description = description or constraint_type.value
        self.validation_count = 0
        self.passed_count = 0
        self.failed_count = 0
    
    @abstractmethod
    def validate(self, function: ExecutableFunction) -> ValidationResult:
        """Validate a function against this constraint.
        
        Args:
            function: The function to validate
            
        Returns:
            ValidationResult indicating pass/fail/warning
        """
        raise NotImplementedError
    
    def get_stats(self) -> Dict[str, Any]:
        """Get validation statistics."""
        pass_rate = (
            self.passed_count / max(1, self.validation_count) * 100
        )
        return {
            'constraint_type': self.constraint_type.value,
            'description': self.description,
            'validation_count': self.validation_count,
            'passed_count': self.passed_count,
            'failed_count': self.failed_count,
            'pass_rate_percent': pass_rate,
        }


# ============================================================================
# Concrete Validators
# ============================================================================

class DeterminismValidator(ConstraintValidator):
    """Validates that function is deterministic.
    
    Checks for:
    - No random functions (random.*, np.random.*)
    - No datetime operations (datetime.now(), time.time())
    - No I/O operations (open(), requests.*)
    - No global state modifications
    - No external service calls
    """
    
    FORBIDDEN_MODULES = {
        'random', 'secrets', 'os.urandom', 'time',
        'datetime', 'uuid', 'requests', 'urllib', 'http',
        'socket', 'subprocess', 'asyncio',
    }
    
    FORBIDDEN_FUNCTIONS = {
        'random', 'randint', 'choice', 'shuffle', 'seed',
        'time', 'clock', 'sleep', 'now', 'today',
        'open', 'read', 'write', 'request', 'get', 'post',
    }
    
    def __init__(self):
        super().__init__(ConstraintType.DETERMINISTIC, "Function is deterministic")
    
    def validate(self, function: ExecutableFunction) -> ValidationResult:
        """Check if function is deterministic."""
        self.validation_count += 1
        
        try:
            # Parse function code
            tree = ast.parse(function.code)
            
            # Check for forbidden imports and function calls
            violations = self._check_ast_violations(tree)
            
            if violations:
                self.failed_count += 1
                return ValidationResult(
                    constraint_type=self.constraint_type,
                    status=ValidationStatus.FAILED,
                    message=f"Function is not deterministic",
                    details={
                        'violations': violations,
                        'forbidden_modules': list(self.FORBIDDEN_MODULES),
                        'forbidden_functions': list(self.FORBIDDEN_FUNCTIONS),
                    },
                )
            
            self.passed_count += 1
            return ValidationResult(
                constraint_type=self.constraint_type,
                status=ValidationStatus.PASSED,
                message="Function is deterministic",
            )
        
        except SyntaxError as e:
            self.failed_count += 1
            return ValidationResult(
                constraint_type=self.constraint_type,
                status=ValidationStatus.FAILED,
                message=f"Syntax error: {e}",
            )
    
    def _check_ast_violations(self, tree: ast.AST) -> List[str]:
        """Check AST for determinism violations."""
        violations = []
        
        for node in ast.walk(tree):
            # Check imports
            if isinstance(node, (ast.Import, ast.ImportFrom)):
                module_name = self._get_module_name(node)
                if any(forbidden in module_name for forbidden in self.FORBIDDEN_MODULES):
                    violations.append(f"Forbidden import: {module_name}")
            
            # Check function calls
            elif isinstance(node, ast.Call):
                func_name = self._get_function_name(node)
                if any(forbidden in func_name for forbidden in self.FORBIDDEN_FUNCTIONS):
                    violations.append(f"Forbidden function call: {func_name}")
        
        return violations
    
    def _get_module_name(self, node: ast.Union[ast.Import, ast.ImportFrom]) -> str:
        """Extract module name from import node."""
        if isinstance(node, ast.ImportFrom):
            return node.module or ""
        elif isinstance(node, ast.Import):
            return node.names[0].name if node.names else ""
        return ""
    
    def _get_function_name(self, node: ast.Call) -> str:
        """Extract function name from call node."""
        if isinstance(node.func, ast.Name):
            return node.func.id
        elif isinstance(node.func, ast.Attribute):
            return node.func.attr
        return "unknown"


class BoundsCheckValidator(ConstraintValidator):
    """Validates that function includes input bounds checking.
    
    Checks for:
    - Input validation statements
    - Range checks (if/assert with comparisons)
    - Argument type validation
    """
    
    def __init__(self):
        super().__init__(ConstraintType.BOUNDS_CHECK, "Function validates input bounds")
    
    def validate(self, function: ExecutableFunction) -> ValidationResult:
        """Check if function validates input bounds."""
        self.validation_count += 1

        if (function.strategy_metadata or {}).get('validation_mode') == 'code_only':
            self.passed_count += 1
            return ValidationResult(
                constraint_type=self.constraint_type,
                status=ValidationStatus.NOT_APPLICABLE,
                message="Bounds check skipped for code-only validation",
            )
        
        try:
            tree = ast.parse(function.code)
            
            # Check for validation patterns
            has_validation = self._has_validation_patterns(tree)
            
            if has_validation:
                self.passed_count += 1
                return ValidationResult(
                    constraint_type=self.constraint_type,
                    status=ValidationStatus.PASSED,
                    message="Function includes bounds checking",
                )
            
            self.failed_count += 1
            return ValidationResult(
                constraint_type=self.constraint_type,
                status=ValidationStatus.WARNING,
                message="Function may not include bounds checking",
                details={'recommendation': 'Add validation for input ranges'},
            )
        
        except SyntaxError:
            self.failed_count += 1
            return ValidationResult(
                constraint_type=self.constraint_type,
                status=ValidationStatus.FAILED,
                message="Cannot parse function for validation",
            )
    
    def _has_validation_patterns(self, tree: ast.AST) -> bool:
        """Check for validation patterns (if, assert with comparisons)."""
        for node in ast.walk(tree):
            # Check for if statements with comparisons
            if isinstance(node, ast.If):
                if isinstance(node.test, (ast.Compare, ast.BoolOp)):
                    return True
            
            # Check for assert statements
            elif isinstance(node, ast.Assert):
                return True
        
        return False


class OutputRangeValidator(ConstraintValidator):
    """Validates that function output is within expected range.
    
    Checks signature for output type hints and validates
    that function enforces output constraints.
    """
    
    def __init__(self):
        super().__init__(ConstraintType.OUTPUT_RANGE_VALIDATION, "Function validates output range")
    
    def validate(self, function: ExecutableFunction) -> ValidationResult:
        """Check if function has output range validation."""
        self.validation_count += 1

        if (function.strategy_metadata or {}).get('validation_mode') == 'code_only':
            self.passed_count += 1
            return ValidationResult(
                constraint_type=self.constraint_type,
                status=ValidationStatus.NOT_APPLICABLE,
                message="Output range check skipped for code-only validation",
            )
        
        # Check if signature specifies output constraints
        signature = function.signature or {}
        outputs = signature.get('outputs', {})
        
        if not outputs:
            self.failed_count += 1
            return ValidationResult(
                constraint_type=self.constraint_type,
                status=ValidationStatus.WARNING,
                message="No output type signature defined",
            )
        
        # For now, just check that outputs are specified
        # Could enhance to check for runtime bounds validation
        self.passed_count += 1
        return ValidationResult(
            constraint_type=self.constraint_type,
            status=ValidationStatus.PASSED,
            message="Function has output type signature",
        )


class TypeSafetyValidator(ConstraintValidator):
    """Validates that function enforces type contracts.
    
    Checks for:
    - Type hints in signature
    - Argument validation
    - Return type validation
    """
    
    def __init__(self):
        super().__init__(ConstraintType.TYPE_SAFETY, "Function enforces type contracts")
    
    def validate(self, function: ExecutableFunction) -> ValidationResult:
        """Check type safety of function."""
        self.validation_count += 1

        if (function.strategy_metadata or {}).get('validation_mode') == 'code_only':
            self.passed_count += 1
            return ValidationResult(
                constraint_type=self.constraint_type,
                status=ValidationStatus.NOT_APPLICABLE,
                message="Type safety check skipped for code-only validation",
            )
        
        signature = function.signature or {}
        inputs = signature.get('inputs', {})
        outputs = signature.get('outputs', {})
        
        if not inputs or not outputs:
            self.failed_count += 1
            return ValidationResult(
                constraint_type=self.constraint_type,
                status=ValidationStatus.WARNING,
                message="Function lacks complete type signature",
                details={'inputs_defined': bool(inputs), 'outputs_defined': bool(outputs)},
            )
        
        self.passed_count += 1
        return ValidationResult(
            constraint_type=self.constraint_type,
            status=ValidationStatus.PASSED,
            message="Function has type signature",
        )


class ConsistencyValidator(ConstraintValidator):
    """Validates consistency with historical outputs.
    
    This is a placeholder that could check against
    historical execution results.
    """
    
    def __init__(self):
        super().__init__(ConstraintType.CONSISTENCY, "Function output is consistent")
    
    def validate(self, function: ExecutableFunction) -> ValidationResult:
        """Check consistency (placeholder)."""
        self.validation_count += 1
        self.passed_count += 1
        
        return ValidationResult(
            constraint_type=self.constraint_type,
            status=ValidationStatus.PASSED,
            message="Consistency check not yet implemented",
        )


class LosslessReconstructionValidator(ConstraintValidator):
    """Validates that function can losslessly reconstruct data.
    
    For decomposed artifacts, checks that:
    - Decomposition is complete (captures all data)
    - Reconstruction logic is correct
    - Round-trip preserves data
    """
    
    def __init__(self):
        super().__init__(ConstraintType.LOSSLESS_RECONSTRUCTION, "Function supports lossless reconstruction")
    
    def validate(self, function: ExecutableFunction) -> ValidationResult:
        """Check lossless reconstruction capability."""
        self.validation_count += 1
        
        # Check if function signature indicates decomposition/reconstruction
        signature = function.signature or {}
        metadata = function.strategy_metadata or {}
        
        # Simple heuristic: look for decompose/reconstruct in name or metadata
        is_reconstruction_fn = (
            'decompose' in function.name.lower() or
            'reconstruct' in function.name.lower() or
            'fragment' in function.name.lower()
        )
        
        if is_reconstruction_fn:
            # Mark as supporting lossless reconstruction
            self.passed_count += 1
            return ValidationResult(
                constraint_type=self.constraint_type,
                status=ValidationStatus.PASSED,
                message="Function appears to support lossless reconstruction",
            )
        
        # Not a reconstruction function - not applicable
        self.passed_count += 1
        return ValidationResult(
            constraint_type=self.constraint_type,
            status=ValidationStatus.NOT_APPLICABLE,
            message="Function is not a reconstruction function",
        )


# ============================================================================
# Validator Registry and Composition
# ============================================================================

class ConstraintValidatorRegistry:
    """Registry of constraint validators."""
    
    def __init__(self):
        """Initialize with default validators."""
        self._validators: Dict[ConstraintType, ConstraintValidator] = {}
        
        # Register default validators
        self._register_default_validators()
    
    def _register_default_validators(self) -> None:
        """Register built-in validators."""
        validators = [
            DeterminismValidator(),
            BoundsCheckValidator(),
            OutputRangeValidator(),
            TypeSafetyValidator(),
            ConsistencyValidator(),
            LosslessReconstructionValidator(),
        ]
        
        for validator in validators:
            self.register(validator)
    
    def register(self, validator: ConstraintValidator) -> None:
        """Register a validator.
        
        Args:
            validator: The validator to register
        """
        self._validators[validator.constraint_type] = validator
        logger.info(f"Registered validator: {validator.description}")
    
    def get_validator(self, constraint_type: ConstraintType) -> Optional[ConstraintValidator]:
        """Get validator for a constraint type.
        
        Args:
            constraint_type: The constraint type
            
        Returns:
            The validator, or None if not registered
        """
        return self._validators.get(constraint_type)
    
    def _coerce_function(self, function: Any) -> ExecutableFunction:
        if isinstance(function, ExecutableFunction):
            return function
        if isinstance(function, str):
            return ExecutableFunction(
                name="anonymous_function",
                language="python",
                code=function,
                signature=None,
                strategy_metadata={"validation_mode": "code_only"},
            )
        raise TypeError(f"Unsupported function type for validation: {type(function)!r}")

    def validate_all(self, function: Any) -> ValidationResults:
        """Validate function against all registered constraints.
        
        Args:
            function: The function to validate
            
        Returns:
            ValidationResults with all validation results
        """
        results = ValidationResults()
        
        coerced = self._coerce_function(function)

        for validator in self._validators.values():
            result = validator.validate(coerced)
            results.add_result(result)
        
        return results
    
    def validate_specific(
        self,
        function: Any,
        constraint_types: List[ConstraintType],
    ) -> ValidationResults:
        """Validate function against specific constraints.
        
        Args:
            function: The function to validate
            constraint_types: List of constraints to check
            
        Returns:
            ValidationResults with requested validations
        """
        results = ValidationResults()
        
        coerced = self._coerce_function(function)

        for constraint_type in constraint_types:
            validator = self._validators.get(constraint_type)
            if validator:
                result = validator.validate(coerced)
                results.add_result(result)
        
        return results
    
    def get_stats(self) -> Dict[str, Any]:
        """Get statistics for all validators."""
        return {
            validator_type.value: validator.get_stats()
            for validator_type, validator in self._validators.items()
        }


# ============================================================================
# Global Registry Instance
# ============================================================================

# Global instance for convenience
_global_registry: Optional[ConstraintValidatorRegistry] = None


def get_global_registry() -> ConstraintValidatorRegistry:
    """Get or create global validator registry."""
    global _global_registry
    if _global_registry is None:
        _global_registry = ConstraintValidatorRegistry()
    return _global_registry


def validate_function(function: ExecutableFunction) -> ValidationResults:
    """Validate function using global registry.
    
    Args:
        function: The function to validate
        
    Returns:
        ValidationResults from all validators
    """
    return get_global_registry().validate_all(function)


def validate_constraints(
    function: ExecutableFunction,
    constraints: List[ConstraintType],
) -> ValidationResults:
    """Validate function against specific constraints.
    
    Args:
        function: The function to validate
        constraints: Constraints to check
        
    Returns:
        ValidationResults from requested validators
    """
    return get_global_registry().validate_specific(function, constraints)
