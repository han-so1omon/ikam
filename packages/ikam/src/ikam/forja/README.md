# Forja Package Migration Plan

**Status:** Scaffold phase (November 2025)  
**Migration Strategy:** Gradual refactor with backward-compatible re-exports

## Overview

The `forja` subpackage houses IKAM's decomposition/reconstruction engine. Currently, it's a **compatibility shim** that re-exports the legacy implementation from `../forja.py` to avoid any behavior changes during the transition period.

## Current Structure

```
narraciones_ikam/
├── forja.py                    # Legacy implementation (active)
└── forja/                      # New subpackage (scaffold)
    ├── __init__.py             # Re-exports from legacy module
    ├── models.py               # Placeholder (empty)
    ├── decomposer.py           # Placeholder (empty)
    ├── reconstructor.py        # Placeholder (empty)
    ├── translator.py           # Placeholder (empty)
    └── radicals/
        └── __init__.py         # Placeholder (empty)
```

## Export Strategy

The `forja/__init__.py` module uses dynamic import to load symbols from the legacy `forja.py` file:

```python
# Dynamically load legacy implementation
from pathlib import Path
import importlib.util

_parent_dir = Path(__file__).resolve().parent.parent
_legacy_path = _parent_dir / "forja.py"

# Load and re-export: decompose_document, reconstruct_document, etc.
```

**Public API preserved:**
- `decompose_document(content, artifact_id, config)`
- `decompose_excel(content, artifact_id, config)`
- `decompose_powerpoint(content, artifact_id, config)`
- `reconstruct_document(fragments, config)`
- `DecompositionError`
- `ReconstructionError`

## Migration Roadmap

### Phase 1: Scaffold (✅ Current)
- Create subpackage structure with empty modules
- Implement re-export shim in `__init__.py`
- Validate no behavior changes via round-trip tests

### Phase 2: Incremental Migration (Upcoming)
- Move decomposition logic → `decomposer.py`
- Move reconstruction logic → `reconstructor.py`
- Keep re-exports in `__init__.py` pointing to new locations
- Add provenance hooks during migration (Task 6)

### Phase 3: Full Migration
- Migrate all helpers and utilities
- Add radical templates to `radicals/`
- Add translator implementations
- Deprecate legacy `forja.py` module

### Phase 4: Cleanup
- Remove legacy `forja.py` file
- Update `__init__.py` to import directly from submodules
- Update all documentation references

## Testing Strategy

All existing tests continue to work without modification:
- `packages/ikam/tests/test_codec_roundtrip.py` (17 tests)
- `packages/ikam/tests/test_fisher_info.py` (17 tests, 1 skipped)

**CI Requirements:**
- Set `PYTHONPATH=packages/ikam/src` for test runs
- Validate API surface remains stable across migration phases
- Maintain 100% pass rate for round-trip guarantees

## Design Principles

1. **Backward Compatibility:** All imports continue to work (`from ikam.forja import decompose_document`)
2. **Zero Behavior Changes:** Re-exports preserve exact runtime behavior during migration
3. **Incremental Migration:** Move code module-by-module, validate at each step
4. **API Stability:** Public exports in `__init__.py` remain stable throughout

## Integration Points

**Current dependencies:**
- `ikam.fragments` — Fragment models, configs
- `ikam.codec` — FragmentCodec for serialization
- `ikam.almacen.chunking` — Optional Excel/PowerPoint chunking

**Future integration (Task 6):**
- `ikam.provenance` — Auto-record DECOMPOSITION edges
- `ikam.fisher_info` — Update FI metrics on decomposition

## References

- [IKAM v2 Specification](../../../../docs/ikam/ikam-v2-fragmented-knowledge-system.md)
- [Fisher Information Gains](../../../../docs/ikam/FISHER_INFORMATION_GAINS.md)
- [Task 5 Completion](../../../../docs/ikam/TASK5_FISHER_INFO_COMPLETE.md)
