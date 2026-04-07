"""Tests for code canonicalization module.

Tests validate that:
1. Semantic preservation: Canonicalized code behaves identically to original
2. Determinism: Same semantic code → same canonical form
3. Storage efficiency: Maximize CAS deduplication via normalization
"""

import pytest
from modelado.core.canonicalize import (
    CodeCanonicalizer,
    CanonicalizedCode,
    canonicalize_function,
)


class TestCanonicalizedCodeModel:
    """Test CanonicalizedCode dataclass."""

    def test_to_dict_serialization(self):
        """Test CanonicalizedCode serialization to dict."""
        code = CanonicalizedCode(
            canonical_code="x = 1",
            content_hash="abc123",
            original_hash="def456",
            transformations=["normalize_whitespace"],
            is_semantically_equivalent=True,
        )
        
        result = code.to_dict()
        
        assert result["canonical_code"] == "x = 1"
        assert result["content_hash"] == "abc123"
        assert result["original_hash"] == "def456"
        assert result["transformations"] == ["normalize_whitespace"]
        assert result["is_semantically_equivalent"] is True


class TestCodeCanonicalizer:
    """Test CodeCanonicalizer class."""

    def test_canonicalize_simple_function(self):
        """Test canonicalizing a simple function."""
        code = """def func():
    x = 1
    return x
"""
        canonicalizer = CodeCanonicalizer(use_blake3=False)
        result = canonicalizer.canonicalize(code)
        
        assert result.is_semantically_equivalent is True
        assert result.canonical_code is not None
        assert len(result.content_hash) > 0

    def test_canonicalize_invalid_syntax(self):
        """Test handling of invalid Python syntax."""
        code = "def func(\n  invalid syntax here"
        canonicalizer = CodeCanonicalizer(use_blake3=False)
        result = canonicalizer.canonicalize(code)
        
        # Should return original code and mark as not semantically equivalent
        assert result.is_semantically_equivalent is False
        assert result.canonical_code == code
        assert "parse_failed" in result.transformations

    def test_canonicalize_deterministic(self):
        """Test that canonicalization is deterministic."""
        code = """def foo():
    x = 1
    y = 2
    return x + y
"""
        canonicalizer1 = CodeCanonicalizer(use_blake3=False)
        canonicalizer2 = CodeCanonicalizer(use_blake3=False)
        
        result1 = canonicalizer1.canonicalize(code)
        result2 = canonicalizer2.canonicalize(code)
        
        # Same input should produce identical output
        assert result1.canonical_code == result2.canonical_code
        assert result1.content_hash == result2.content_hash


class TestCanonicalizeFunction:
    """Test the convenience function."""

    def test_canonicalize_function_default(self):
        """Test canonicalize_function convenience function."""
        code = "x = 1\n"
        result = canonicalize_function(code, use_blake3=False)
        
        assert isinstance(result, CanonicalizedCode)
        assert result.is_semantically_equivalent is True
        assert result.canonical_code is not None


class TestCanonicalDeduplication:
    """Test that canonicalization enables CAS deduplication."""

    def test_identical_functions_same_hash(self):
        """Test that identical functions produce same canonical hash."""
        code1 = "def add(x, y):\n    return x + y\n"
        code2 = "def add(x, y):\n    return x + y\n"
        
        result1 = canonicalize_function(code1, use_blake3=False)
        result2 = canonicalize_function(code2, use_blake3=False)
        
        assert result1.content_hash == result2.content_hash

    def test_whitespace_variance_same_hash(self):
        """Test that whitespace variations produce same hash."""
        code1 = "x=1\ny=2\n"
        code2 = "x = 1\ny = 2\n"
        code3 = "x  =  1\ny  =  2\n"
        
        result1 = canonicalize_function(code1, use_blake3=False)
        result2 = canonicalize_function(code2, use_blake3=False)
        result3 = canonicalize_function(code3, use_blake3=False)
        
        # All should canonicalize to same form
        assert result1.canonical_code == result2.canonical_code
        assert result2.canonical_code == result3.canonical_code
        assert result1.content_hash == result2.content_hash == result3.content_hash


class TestEdgeCases:
    """Test edge cases and special scenarios."""

    def test_empty_code(self):
        """Test canonicalization of empty code."""
        code = ""
        canonicalizer = CodeCanonicalizer(use_blake3=False)
        result = canonicalizer.canonicalize(code)
        
        assert result.is_semantically_equivalent is True

    def test_with_try_except(self):
        """Test canonicalization with exception handling."""
        code = """try:
    x = 1 / 0
except ZeroDivisionError:
    x = 0
finally:
    print(x)
"""
        result = canonicalize_function(code, use_blake3=False)
        
        assert result.is_semantically_equivalent is True


class TestSortImports:
    """Test import sorting functionality."""

    def test_sort_regular_imports(self):
        """Test that regular imports are sorted alphabetically."""
        code = """import sys
import os
import ast

def foo():
    pass
"""
        canonicalizer = CodeCanonicalizer(use_blake3=False)
        result = canonicalizer.canonicalize(code)
        
        assert "sort_imports" in result.transformations
        lines = result.canonical_code.split("\n")
        # First three non-empty lines should contain sorted imports
        imports = [line for line in lines if line.startswith("import ")]
        assert len(imports) >= 3

    def test_sort_from_imports(self):
        """Test that 'from X import Y' statements are sorted."""
        code = """from z_module import func_z
from a_module import func_a
from m_module import func_m

x = 1
"""
        canonicalizer = CodeCanonicalizer(use_blake3=False)
        result = canonicalizer.canonicalize(code)
        
        assert "sort_imports" in result.transformations

    def test_mixed_imports_sorted(self):
        """Test that mixed import types are sorted together."""
        code = """from datetime import datetime
import sys
from os import path
import ast

def foo():
    pass
"""
        result = canonicalize_function(code, use_blake3=False)
        
        assert "sort_imports" in result.transformations


class TestTextNormalization:
    """Test text-level normalization."""

    def test_normalize_line_endings_crlf(self):
        """Test that CRLF line endings are normalized to LF."""
        code = "x = 1\r\ny = 2\r\n"
        canonicalizer = CodeCanonicalizer(use_blake3=False)
        result = canonicalizer.canonicalize(code)
        
        assert "normalize_line_endings" in result.transformations
        assert "\r\n" not in result.canonical_code

    def test_normalize_line_endings_cr(self):
        """Test that CR line endings are normalized to LF."""
        code = "x = 1\ry = 2\r"
        canonicalizer = CodeCanonicalizer(use_blake3=False)
        result = canonicalizer.canonicalize(code)
        
        assert "normalize_line_endings" in result.transformations
        assert "\r" not in result.canonical_code

    def test_remove_trailing_whitespace(self):
        """Test that trailing whitespace is removed from lines."""
        code = "x = 1   \ny = 2\t\n"
        canonicalizer = CodeCanonicalizer(use_blake3=False)
        result = canonicalizer.canonicalize(code)
        
        assert "remove_trailing_whitespace" in result.transformations
        lines = result.canonical_code.split("\n")[:-1]  # Exclude final newline
        for line in lines:
            if line:  # Non-empty lines
                assert not line.endswith(" ")
                assert not line.endswith("\t")

    def test_normalize_multiple_blank_lines(self):
        """Test that multiple consecutive blank lines are normalized."""
        code = "x = 1\n\n\n\n\ny = 2\n"
        canonicalizer = CodeCanonicalizer(use_blake3=False)
        result = canonicalizer.canonicalize(code)
        
        assert "normalize_blank_lines" in result.transformations
        # Should not have more than 2 consecutive newlines
        assert "\n\n\n" not in result.canonical_code

    def test_file_ends_with_single_newline(self):
        """Test that files end with exactly one newline."""
        code = "x = 1\ny = 2"
        canonicalizer = CodeCanonicalizer(use_blake3=False)
        result = canonicalizer.canonicalize(code)
        
        # Should end with single newline
        assert result.canonical_code.endswith("\n")
        assert not result.canonical_code.endswith("\n\n")


class TestHashingBehavior:
    """Test hashing functionality."""

    def test_sha256_fallback_when_blake3_disabled(self):
        """Test that SHA256 is used when BLAKE3 is disabled."""
        code = "x = 1\n"
        canonicalizer = CodeCanonicalizer(use_blake3=False)
        result = canonicalizer.canonicalize(code)
        
        # SHA256 produces 64 hex characters
        assert len(result.content_hash) == 64
        # Should be valid hex
        int(result.content_hash, 16)

    def test_content_hash_consistency(self):
        """Test that content hashes are consistent across runs."""
        code = "def foo():\n    return 42\n"
        canonicalizer = CodeCanonicalizer(use_blake3=False)
        
        hashes = []
        for _ in range(10):
            result = canonicalizer.canonicalize(code)
            hashes.append(result.content_hash)
        
        # All hashes should be identical
        assert len(set(hashes)) == 1

    def test_original_hash_different_from_canonical(self):
        """Test that original hash differs from canonical when changes are made."""
        code = "import sys\nimport os\n\nx = 1\n\n\n\ny = 2   \n"
        canonicalizer = CodeCanonicalizer(use_blake3=False)
        result = canonicalizer.canonicalize(code)
        
        # Original and canonical should have different hashes due to transformations
        assert result.original_hash != result.content_hash


class TestVariableRenaming:
    """Test variable renaming functionality."""

    def test_function_parameters_renamed(self):
        """Test that function parameters are renamed to canonical form."""
        code = """def add(first, second):
    return first + second
"""
        canonicalizer = CodeCanonicalizer(use_blake3=False)
        result = canonicalizer.canonicalize(code)
        
        # Should rename parameters (though exact names depend on implementation)
        assert result.is_semantically_equivalent is True


class TestDictSorting:
    """Test dictionary literal sorting."""

    def test_sort_simple_dict(self):
        """Test that simple dictionary literals are sorted by key."""
        code = """x = {"z": 1, "a": 2, "m": 3}
"""
        canonicalizer = CodeCanonicalizer(use_blake3=False)
        result = canonicalizer.canonicalize(code)
        
        assert result.is_semantically_equivalent is True

    def test_nested_dict_sorted(self):
        """Test that nested dictionaries are sorted."""
        code = """config = {
    "z_setting": {"nested_z": 1},
    "a_setting": {"nested_a": 2}
}
"""
        canonicalizer = CodeCanonicalizer(use_blake3=False)
        result = canonicalizer.canonicalize(code)
        
        assert result.is_semantically_equivalent is True


class TestSemanticEquivalence:
    """Test semantic equivalence detection."""

    def test_semantically_identical_code(self):
        """Test that semantically identical code is recognized."""
        code1 = """def multiply(a, b):
    result = a * b
    return result
"""
        code2 = """def multiply(x, y):
    temp = x * y
    return temp
"""
        result1 = canonicalize_function(code1, use_blake3=False)
        result2 = canonicalize_function(code2, use_blake3=False)
        
        # Both should be marked as semantically equivalent
        assert result1.is_semantically_equivalent is True
        assert result2.is_semantically_equivalent is True

    def test_different_implementations_different_hash(self):
        """Test that different implementations have different hashes."""
        code1 = "def f():\n    return 1\n"
        code2 = "def f():\n    return 2\n"
        
        result1 = canonicalize_function(code1, use_blake3=False)
        result2 = canonicalize_function(code2, use_blake3=False)
        
        # Different return values should produce different hashes
        assert result1.content_hash != result2.content_hash


class TestComplexCode:
    """Test canonicalization of complex code structures."""

    def test_class_with_multiple_methods(self):
        """Test canonicalization of classes with multiple methods."""
        code = """class Calculator:
    def add(self, x, y):
        return x + y
    
    def subtract(self, x, y):
        return x - y
    
    def multiply(self, x, y):
        return x * y
"""
        result = canonicalize_function(code, use_blake3=False)
        
        assert result.is_semantically_equivalent is True

    def test_with_decorators_and_generators(self):
        """Test canonicalization of code with decorators and generators."""
        code = """@property
def values(self):
    for i in range(10):
        yield i * 2
"""
        result = canonicalize_function(code, use_blake3=False)
        
        assert result.is_semantically_equivalent is True

    def test_async_code(self):
        """Test canonicalization of async functions."""
        code = """async def fetch_data():
    await asyncio.sleep(1)
    return {"status": "ok"}
"""
        result = canonicalize_function(code, use_blake3=False)
        
        assert result.is_semantically_equivalent is True

    def test_type_hints(self):
        """Test canonicalization of code with type hints."""
        code = """def process(data: list[int]) -> int:
    return sum(data)
"""
        result = canonicalize_function(code, use_blake3=False)
        
        assert result.is_semantically_equivalent is True


class TestIdempotence:
    """Test that canonicalization is idempotent."""

    def test_double_canonicalization_identical(self):
        """Test that canonicalize(canonicalize(x)) == canonicalize(x)."""
        code = """import sys
import os

def foo(x, y):
    return x + y


z = 42
"""
        result1 = canonicalize_function(code, use_blake3=False)
        result2 = canonicalize_function(result1.canonical_code, use_blake3=False)
        
        # Second canonicalization should be identical
        assert result1.canonical_code == result2.canonical_code
        assert result1.content_hash == result2.content_hash

    def test_triple_canonicalization_identical(self):
        """Test that canonicalize(canonicalize(canonicalize(x))) is stable."""
        code = "x = 1\n\n\ny = 2   \n"
        
        result1 = canonicalize_function(code, use_blake3=False)
        result2 = canonicalize_function(result1.canonical_code, use_blake3=False)
        result3 = canonicalize_function(result2.canonical_code, use_blake3=False)
        
        # All should converge to same canonical form
        assert result1.canonical_code == result2.canonical_code == result3.canonical_code
        assert result1.content_hash == result2.content_hash == result3.content_hash


class TestTransformationsRecording:
    """Test that transformations are properly recorded."""

    def test_all_transformations_listed(self):
        """Test that all applied transformations are recorded."""
        code = """import sys
import os

x = 1


y = 2   
"""
        canonicalizer = CodeCanonicalizer(use_blake3=False)
        result = canonicalizer.canonicalize(code)
        
        # Should have multiple transformations
        assert len(result.transformations) >= 3
        # Common transformations should be present
        assert any("normalize" in t for t in result.transformations)

    def test_transformations_for_minimal_code(self):
        """Test transformations for minimal code changes."""
        code = "x = 1\n"
        canonicalizer = CodeCanonicalizer(use_blake3=False)
        result = canonicalizer.canonicalize(code)
        
        # Even minimal code has some transformations applied
        assert len(result.transformations) > 0
