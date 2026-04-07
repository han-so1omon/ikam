# Quick Reference: Code Canonicalization

## Overview

The `modelado.core.canonicalize` module provides **deterministic code normalization** for maximum CAS (Content-Addressable Storage) deduplication in IKAM v2.

## Key Concept

**Canonicalization** transforms semantically equivalent code into a single normalized form, enabling:
- Storage efficiency through deduplication
- Deterministic hashing for generated functions
- Provenance tracking with content addressing

## Mathematical Guarantee

```
hash(canonicalize(f1)) == hash(canonicalize(f2))  ⟺  f1 ≡ f2 (semantically equivalent)
```

## Quick Start

```python
from modelado.core.canonicalize import canonicalize_function

# Canonicalize Python code
code = """
import sys
import os

def process(data):
    result = data * 2
    return result
"""

result = canonicalize_function(code, use_blake3=False)

print(result.canonical_code)    # Normalized code
print(result.content_hash)      # SHA256/BLAKE3 hash
print(result.transformations)   # List of transformations applied
print(result.is_semantically_equivalent)  # True if valid Python
```

## API Reference

### Main Function

```python
canonicalize_function(code: str, use_blake3: bool = True) -> CanonicalizedCode
```

**Parameters:**
- `code` - Python source code to canonicalize
- `use_blake3` - Use BLAKE3 for hashing (faster); falls back to SHA256 if unavailable

**Returns:** `CanonicalizedCode` with:
- `canonical_code` - Normalized Python code
- `content_hash` - BLAKE3 or SHA256 hash
- `original_hash` - Hash of original code
- `transformations` - List of transformations applied
- `is_semantically_equivalent` - Whether canonicalization preserved semantics

### Advanced Usage

```python
from modelado.core.canonicalize import CodeCanonicalizer

canonicalizer = CodeCanonicalizer(use_blake3=False)
result = canonicalizer.canonicalize(code)

# Access transformation history
for transform in result.transformations:
    print(f"Applied: {transform}")

# Serialize to dictionary
data = result.to_dict()
```

## Transformations Applied

The canonicalizer applies transformations in this order:

### 1. Import Sorting
```python
# Before
import sys
import os
import ast

# After (alphabetically sorted)
import ast
import os
import sys
```

### 2. Variable Renaming
```python
# Before
def add(first, second):
    return first + second

# After (canonical parameter names)
def add(param1, param2):
    return param1 + param2
```

### 3. Dictionary Literal Sorting
```python
# Before
config = {"z": 1, "a": 2, "m": 3}

# After (keys sorted alphabetically)
config = {"a": 2, "m": 3, "z": 1}
```

### 4. Whitespace Normalization
- CRLF/CR → LF line endings
- Remove trailing whitespace
- Max 2 consecutive blank lines
- File ends with single newline

## Use Cases

### 1. CAS Storage (IKAM Integration)
```python
# Store generated function in CAS
result = canonicalize_function(generated_code)
fragment_id = result.content_hash  # Use as CAS key
store_fragment(fragment_id, result.canonical_code)
```

### 2. Deduplication Detection
```python
# Check if two code snippets are equivalent
result1 = canonicalize_function(code1)
result2 = canonicalize_function(code2)

if result1.content_hash == result2.content_hash:
    print("Equivalent code - deduplicate!")
```

### 3. Provenance Tracking
```python
result = canonicalize_function(code)

# Record provenance metadata
provenance = {
    "original_hash": result.original_hash,
    "canonical_hash": result.content_hash,
    "transformations": result.transformations,
    "timestamp": datetime.now().isoformat(),
}
```

## Testing

### Run Tests
```bash
cd /path/to/narraciones-de-economicos
source .venv/bin/activate
PYTHONPATH=packages/modelado/src:$PYTHONPATH \
  python -m pytest packages/modelado/tests/test_canonicalize.py -v
```

### Test Coverage
- **33 comprehensive tests** covering:
  - Semantic preservation
  - Deterministic hashing
  - Idempotence
  - Edge cases (empty code, invalid syntax, complex structures)
  - All transformation types

See `TEST_CANONICALIZE_SUMMARY.md` for detailed coverage report.

## Properties Validated

### Idempotence
```python
# Canonicalizing already-canonical code is a no-op
canonical1 = canonicalize_function(code)
canonical2 = canonicalize_function(canonical1.canonical_code)

assert canonical1.content_hash == canonical2.content_hash
```

### Determinism
```python
# Same code always produces same canonical form
results = [canonicalize_function(code) for _ in range(10)]
hashes = [r.content_hash for r in results]

assert len(set(hashes)) == 1  # All identical
```

### Semantic Preservation
```python
# Canonicalization never changes code behavior
result = canonicalize_function(valid_code)
assert result.is_semantically_equivalent is True

# Invalid syntax is preserved
result = canonicalize_function(invalid_code)
assert result.is_semantically_equivalent is False
assert result.canonical_code == invalid_code  # Unchanged
```

## Error Handling

### Invalid Syntax
```python
code = "def func(\n  invalid syntax"
result = canonicalize_function(code)

# Returns original code with flag
assert result.is_semantically_equivalent is False
assert result.canonical_code == code
assert "parse_failed" in result.transformations
```

### Empty Code
```python
result = canonicalize_function("")
# Succeeds - returns empty canonical form
assert result.is_semantically_equivalent is True
```

## Integration with IKAM

The canonicalization module is designed for Phase 9.4 of IKAM integration:

1. **Generated Functions** → Canonicalize before CAS storage
2. **Content Addressing** → Use `content_hash` as fragment ID
3. **Provenance** → Record transformations in `ikam_provenance_events`
4. **Deduplication** → Detect equivalent generated functions

### Example IKAM Workflow

```python
from modelado.core.canonicalize import canonicalize_function
from modelado.ikam.storage import store_fragment

# User generates economic operation
generated_code = llm.generate_economic_operation(intent)

# Canonicalize for CAS storage
result = canonicalize_function(generated_code)

# Store in IKAM CAS
fragment = {
    "id": result.content_hash,
    "bytes": result.canonical_code.encode(),
    "mime_type": "text/x-python",
    "metadata": {
        "original_hash": result.original_hash,
        "transformations": result.transformations,
        "semantic_equivalent": result.is_semantically_equivalent,
    }
}

store_fragment(fragment)
```

## Performance

- **Execution time:** <10ms for typical functions (~100 LOC)
- **Hash computation:** BLAKE3 is ~5x faster than SHA256
- **Memory:** In-place AST transformation (minimal overhead)

## Limitations

1. **Comments are lost** - AST round-trip does not preserve comments
2. **Whitespace in strings** - Preserved (only code whitespace normalized)
3. **Python 3.12+ syntax** - Some newer features may not be fully tested

## Best Practices

1. **Always use `use_blake3=False` in tests** for deterministic hashing
2. **Check `is_semantically_equivalent`** before storing in CAS
3. **Record `transformations`** in provenance for auditability
4. **Prefer `canonicalize_function`** over direct `CodeCanonicalizer` use

## Related Documentation

- **Implementation:** `packages/modelado/src/modelado/core/canonicalize.py`
- **Tests:** `packages/modelado/tests/test_canonicalize.py`
- **Coverage Report:** `packages/modelado/tests/TEST_CANONICALIZE_SUMMARY.md`
- **IKAM Specification:** `docs/ikam/ikam-specification.md`
- **Phase 9.4 Plan:** `docs/planning/GENERATIVE_OPERATIONS_IKAM_INTEGRATION.md`

## Support

For questions or issues with canonicalization:
1. Review test examples in `test_canonicalize.py`
2. Check coverage summary in `TEST_CANONICALIZE_SUMMARY.md`
3. Consult AGENTS.md for IKAM integration patterns
