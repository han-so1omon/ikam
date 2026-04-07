# IKAM v2 FragmentCodec & Decomposition

**Status:** MVP Implementation Complete (November 16, 2025)  
**Version:** 1.0.0  
**Dependencies:** `pydantic>=2.7`, optional: `blake3>=0.4`, `ikam[chunking]`

## Overview

The FragmentCodec and Forja (decomposition/reconstruction) modules implement IKAM v2's mathematical guarantees for lossless knowledge fragmentation. These are the Day 2 deliverables of the 2-week IKAM v2 MVP.

### Mathematical Guarantees

1. **Lossless Codec:** `decode(encode(F)) = F` (byte-level equality)
2. **Deterministic Encoding:** `encode(F) = encode(F)` (idempotent)
3. **Canonical Uniqueness:** `encode(F1) = encode(F2)` iff `F1 = F2`
4. **Lossless Reconstruction:** `reconstruct(decompose(A)) = A` (content preservation)
5. **Monotone Refinement:** `F_k ⊆ F_{k+1}` (fragments added, never removed)

See `docs/ikam/ikam-fragmentation-math-model.md` for formal proofs.

## Architecture

### File Structure

```
packages/ikam/src/narraciones_ikam/
├── fragments.py          # Fragment domain models (Pydantic)
├── codec.py              # FragmentCodec (encode/decode/validate)
├── forja.py              # Decomposition/reconstruction (decompose/reconstruct)
├── almacen/              # Storage backend (from Day 1)
│   ├── postgres.py       # PostgreSQL CAS storage
│   ├── chunking.py       # Document chunking utilities
│   └── ...
└── __init__.py           # Public API exports
```

### Fragment Model

```python
from narraciones_ikam import Fragment, FragmentType, TextFragmentContent, TextBlock

fragment = Fragment(
    id="frag-abc-001",
    artifact_id="artifact-123",
    level=0,  # Depth within a view-specific decomposition tree
    type=FragmentType.TEXT,
    content=TextFragmentContent(
        summary="Executive summary",
        blocks=[TextBlock(type="paragraph", content="Core value proposition...")],
    ),
    salience=1.0,  # Hot tier (0.0-1.0)
)
```

**Fragment Types:**
- `TEXT`: Text blocks (paragraphs, headings, lists)
- `DATA`: Tabular data with schema
- `VISUAL`: Charts (Vega-Lite specs) or images
- `STRUCTURAL`: Layout containers with child fragments

**Hierarchy Levels:**
- `level` is **depth within a specific decomposition view**, not a global abstraction ladder.
- Depth meaning is defined by the active decomposition plan (see `docs/ikam/SEMANTIC_FIRST_DECOMPOSITION.md`).
- Salience and render policies determine what gets surfaced; levels are not semantic labels.

Salience drives storage tier routing (hot/warm/cold) via `almacén.TieredStorage`.

## FragmentCodec API

### Basic Usage

```python
from narraciones_ikam import FragmentCodec, Fragment

codec = FragmentCodec(compress=False, validate_on_decode=True)

# Encode (Fragment → bytes)
encoded_bytes = codec.encode(fragment)

# Decode (bytes → Fragment)
decoded_fragment = codec.decode(encoded_bytes)

# Validate integrity
is_valid = codec.validate(encoded_bytes)  # True if lossless round-trip succeeds

# Content hash (for CAS addressing)
fragment_hash = codec.hash(fragment)  # BLAKE3 or SHA256 hex (64 chars)
```

### Compression

```python
# Enable gzip compression for large fragments
codec = FragmentCodec(compress=True)
encoded = codec.encode(fragment)  # Gzip-compressed bytes

# Codec auto-detects gzip on decode (no configuration needed)
decoded = codec.decode(encoded)
```

### Batch Encoding (FragmentListCodec)

```python
from narraciones_ikam import FragmentListCodec

codec = FragmentListCodec(compress=False)

fragments = [fragment1, fragment2, fragment3]

# Encode list
encoded = codec.encode(fragments)  # JSON array of fragments

# Decode list
decoded_fragments = codec.decode(encoded)

# Validate list integrity
is_valid = codec.validate(encoded)
```

## Decomposition & Reconstruction (Forja)

### Document Decomposition

```python
from narraciones_ikam import decompose_document, DecompositionConfig

text = """
# Executive Summary
This is a venture pitch...

## Problem Statement
The market lacks...
"""

config = DecompositionConfig(
    strategy="semantic",
    target_levels=3,  # Depth cap (view-specific), not a semantic tier
    max_tokens_per_fragment=700,
    overlap_tokens=150,
)

fragments = decompose_document(
    content=text,
    artifact_id="artifact-doc-001",
    config=config,
)

# Returns List[Fragment] ordered by (level, position)
# Depth 0..N reflect view-specific structure (example only; not a fixed semantic ladder)
```

### Excel & PowerPoint Decomposition

Requires `ikam[chunking]` installed.

```python
from narraciones_ikam import decompose_excel, decompose_powerpoint

# Excel: one fragment per sheet
excel_bytes = open("model.xlsx", "rb").read()
fragments = decompose_excel(excel_bytes, artifact_id="artifact-excel-001")

# PowerPoint: one fragment per slide
pptx_bytes = open("pitch.pptx", "rb").read()
fragments = decompose_powerpoint(pptx_bytes, artifact_id="artifact-pptx-001")
```

### Document Reconstruction

```python
from narraciones_ikam import reconstruct_document, ReconstructionConfig

config = ReconstructionConfig(
    render_levels=[0, 1],  # Depth filter (view-specific); use render policy for semantics
    target_format="ikam-document",
    validate_integrity=True,
)

reconstructed_text = reconstruct_document(fragments, config)

# Returns reconstructed document text
# Lossless: original content preserved (modulo formatting normalization)
```

## Storage Integration (Almacén)

Wire fragments to PostgreSQL CAS storage:

```python
from narraciones_ikam.almacen import PostgresBackend, FragmentRecord, FragmentKey
from narraciones_ikam import FragmentCodec

backend = PostgresBackend(
    connection_string="postgresql://user:pass@localhost/db",
    auto_initialize=True,
)

codec = FragmentCodec(compress=True)

# Store fragment
encoded = codec.encode(fragment)
fragment_hash = codec.hash(fragment)

key = backend.put(FragmentRecord(
    key=FragmentKey(key=f"blake3:{fragment_hash}", kind="text"),
    payload=encoded,
    metadata={"level": fragment.level, "salience": fragment.salience},
))

# Retrieve fragment
record = backend.get(key)
if record:
    decoded_fragment = codec.decode(record.payload)
```

See `packages/ikam/ALMACEN_README.md` for full almacén documentation.

## Testing

### Running Tests

```bash
# All round-trip tests
pytest packages/ikam/tests/test_codec_roundtrip.py -v

# Specific test class
pytest packages/ikam/tests/test_codec_roundtrip.py::TestFragmentCodec -v

# Single test
pytest packages/ikam/tests/test_codec_roundtrip.py::TestFragmentCodec::test_lossless_round_trip_uncompressed -v
```

### Test Coverage

**17 tests** validating:
1. Codec losslessness (uncompressed, compressed)
2. Codec determinism (idempotent encoding)
3. Codec uniqueness (different fragments → different bytes)
4. Integrity validation (correct vs corrupted)
5. Hash consistency (deterministic hashing)
6. Batch encoding (FragmentListCodec)
7. Decomposition completeness (produces fragments)
8. Fragment hierarchy depth (view-specific)
9. Reconstruction content preservation
10. Partial reconstruction (depth filtering)
11. Decomposition determinism
12. Edge cases (empty documents)
13. Full pipeline round-trip (document → fragments → bytes → fragments → document)

**Expected Results:**
```
test_codec_roundtrip.py::TestFragmentCodec::test_lossless_round_trip_uncompressed PASSED
test_codec_roundtrip.py::TestFragmentCodec::test_lossless_round_trip_compressed PASSED
test_codec_roundtrip.py::TestFragmentCodec::test_deterministic_encoding PASSED
test_codec_roundtrip.py::TestFragmentCodec::test_unique_encoding_for_different_fragments PASSED
test_codec_roundtrip.py::TestFragmentCodec::test_validate_correct_encoding PASSED
test_codec_roundtrip.py::TestFragmentCodec::test_validate_corrupted_encoding PASSED
test_codec_roundtrip.py::TestFragmentCodec::test_hash_consistency PASSED
test_codec_roundtrip.py::TestFragmentCodec::test_hash_uniqueness PASSED
test_codec_roundtrip.py::TestFragmentListCodec::test_lossless_round_trip_list PASSED
test_codec_roundtrip.py::TestFragmentListCodec::test_validate_fragment_list PASSED
test_codec_roundtrip.py::TestDecompositionReconstruction::test_document_decomposition_produces_fragments PASSED
test_codec_roundtrip.py::TestDecompositionReconstruction::test_fragment_hierarchy_levels PASSED
test_codec_roundtrip.py::TestDecompositionReconstruction::test_reconstruction_preserves_content PASSED
test_codec_roundtrip.py::TestDecompositionReconstruction::test_partial_reconstruction_by_level PASSED
test_codec_roundtrip.py::TestDecompositionReconstruction::test_decomposition_determinism PASSED
test_codec_roundtrip.py::TestDecompositionReconstruction::test_empty_document_decomposition PASSED
test_codec_roundtrip.py::TestIntegrationRoundTrip::test_full_pipeline_round_trip PASSED

17 passed in X.XXs
```

## Installation

### Core Codec (no optional deps)

```bash
pip install ikam
```

### With CAS Storage (PostgreSQL)

```bash
pip install ikam[postgres]
```

### With Document Chunking (Excel/PPTX)

```bash
pip install ikam[chunking]
```

### Full Stack

```bash
pip install ikam[almacen]  # postgres + chunking
```

## Next Steps (Day 3+)

### Day 3: REP (Radicals Encoding Protocol)

Implement multi-level fragment representations:
- REP envelope format (metadata + levels array)
- r4096 encoding (25% smaller than Base64)
- L0 (core ideas) / L1 (full content) / L2 (citations) extractors
- CAS references vs inline content logic (threshold: 256 bytes)

### Day 4: Storage Metrics + Observability

Validate storage monotonicity:
- Prometheus metrics (`narraciones_ikam_delta_bytes`)
- Diagnostics endpoint: `GET /api/diagnostics/ikam/storage`
- Grafana dashboard showing Δ(N) monotonicity
- Break-even analysis (N ≥ 2 validation)

### Day 5: Fisher Information Validation

Prove provenance completeness theorem:
- Store full derivation graphs (fragment → source mapping)
- Provenance queries: `get_derivations(fragment_id)`
- Conditional FI tests: `I(fragment | provenance) ≥ I(fragment)`
- RMSE comparisons vs baseline RAG (≥15% lower RMSE target)

## References

- **IKAM v2 Spec:** `docs/ikam/ikam-v2-fragmented-knowledge-system.md`
- **Math Model:** `docs/ikam/ikam-fragmentation-math-model.md`
- **Storage Gains:** `docs/ikam/STORAGE_GAINS_EXAMPLE.md`
- **Almacén (Day 1):** `packages/ikam/ALMACEN_README.md`
- **Two-Week MVP Plan:** `docs/planning/PHASES_SPRINTS_ROADMAP.md` (Phase 2, IKAM v2 section)

## Changelog

### 1.0.0 (November 16, 2025)

- ✅ Fragment domain models (`fragments.py`)
- ✅ FragmentCodec with lossless encode/decode/validate (`codec.py`)
- ✅ FragmentListCodec for batch operations
- ✅ Document decomposition (`decompose_document`)
- ✅ Excel/PowerPoint decomposition (requires chunking)
- ✅ Document reconstruction (`reconstruct_document`)
- ✅ 17 comprehensive round-trip tests (100% pass rate)
- ✅ BLAKE3 content hashing for CAS addressing
- ✅ Gzip compression support
- ✅ Integration with almacén PostgreSQL backend

**Acceptance Criteria Met:**
- ✅ Round-trip test 100% pass rate
- ✅ No data loss in decomposition/reconstruction
- ✅ Byte-level equality for codec operations
- ✅ Wired to almacén for CAS storage
- ✅ Comprehensive README documentation
