# IKAM Almacén: Content-Addressable Storage

**Status:** ✅ Foundation Complete (Day 1 of IKAM v2 MVP)  
**Date:** November 16, 2025

## Overview

Almacén provides content-addressable storage (CAS) for IKAM v2 fragmentation with automatic deduplication and lossless round-trip guarantees. The PostgreSQL backend implements the full `StorageBackend` interface with BLAKE3 hashing for fragment identification.

## Architecture

```
packages/ikam/almacen/
├── base.py           # StorageBackend interface + FragmentKey/Record
├── registry.py       # BackendRegistry for pluggable engines
├── policy.py         # TierPolicy + TieredStorage coordination
├── postgres.py       # PostgreSQL backend with CAS (NEW ✅)
└── chunking.py       # Document chunking utilities (NEW ✅)
```

## Mathematical Guarantees

### 1. Content-Addressable Storage (CAS)
```
hash(content) → unique key
put(X) → key_X where key_X = blake3(X)
```
- **Collision probability:** < 2^-128 (BLAKE3 cryptographic strength)
- **Idempotence:** `put(X)` returns same key regardless of call count

### 2. Lossless Reconstruction
```
reconstruct(decompose(X)) = X
get(put(X)) = X
```
- **Byte-level equality:** 100% requirement (no lossy compression)
- **Round-trip test:** All tests must pass at 100%

### 3. Storage Monotonicity
```
Δ(N) = S_flat(N) - S_CAS(N)
Δ(N+1) - Δ(N) = s·B - r̄ ≥ 0
```
- **Break-even:** N ≥ 2 for typical venture content (s=0.7, B=5KB, r̄=200B)
- **Deduplication:** Shared fragments stored once, referenced by hash

## PostgreSQL Backend

### Database Schema
```sql
CREATE TABLE ikam_fragments (
    fragment_id SERIAL PRIMARY KEY,
    content_hash TEXT NOT NULL UNIQUE,  -- BLAKE3 hash (CAS key)
    kind TEXT NOT NULL,                 -- fragment kind (text, patch, chart)
    payload BYTEA NOT NULL,             -- raw fragment bytes
    size_bytes INT NOT NULL,            -- payload size for metrics
    metadata JSONB DEFAULT '{}',        -- extensible metadata
    created_at TIMESTAMP DEFAULT NOW(),
    updated_at TIMESTAMP DEFAULT NOW()
);

CREATE INDEX idx_ikam_fragments_hash ON ikam_fragments(content_hash);
CREATE INDEX idx_ikam_fragments_kind ON ikam_fragments(kind);
CREATE INDEX idx_ikam_fragments_metadata ON ikam_fragments USING gin(metadata);
```

### Usage Example
```python
from ikam.almacen import FragmentRecord, FragmentKey
from ikam.almacen.postgres import PostgresBackend

# Initialize backend
backend = PostgresBackend(
    os.getenv("DATABASE_URL", "postgresql://user:pass@localhost:5432/app"),
    auto_initialize=True,  # Creates schema if needed
)

# Store fragment (CAS deduplication automatic)
key = backend.put(FragmentRecord(
    key=FragmentKey(key="", kind="text"),  # key auto-generated
    payload=b"Hello, IKAM v2!",
    metadata={"source": "user-input"},
))
print(f"Stored as: {key.key}")  # blake3:abc123...

# Retrieve fragment
record = backend.get(key)
assert record.payload == b"Hello, IKAM v2!"

# List fragments by kind
text_fragments = list(backend.list(prefix="text"))
```

## Document Chunking

Chunking utilities support Excel and PowerPoint fragmentation with deterministic boundaries and token estimation:

```python
from ikam.almacen.chunking import build_excel_chunks

chunks = build_excel_chunks(
    excel_bytes,
    max_tokens=700,
    overlap=150,
)

for chunk in chunks:
    print(f"Sheet: {chunk.meta['sheet']}, Tokens: {chunk.token_count}")
    print(f"Content: {chunk.content[:100]}...")
```

### Chunking Guarantees
- **Deterministic:** Same input → same chunks
- **Token monotonicity:** Longer content → more tokens
- **Forward progress:** Always advances by ≥1 line per chunk
- **Overlap preservation:** Context maintained across boundaries

## Backward Compatibility

The `ikam.almacen.chunking` module provides content-addressable chunking:

```python
# Standard import (all new code)
from ikam.almacen.chunking import Chunk, build_excel_chunks
```

## Testing

### Round-Trip Tests
```bash
# Run from workspace root
pytest packages/ikam/tests/test_almacen_roundtrip.py -v

# Expected output:
# ✓ test_cas_same_content_same_key
# ✓ test_lossless_round_trip
# ✓ test_idempotent_put
# ✓ test_storage_deduplication_savings
# ✓ test_delete_operation
# ✓ test_list_with_filter
# ✓ test_backend_capabilities
# ✓ test_backend_registry
```

All tests must pass at 100% for Day 1 acceptance.

## Installation

```bash
# Core almacén (interface only)
pip install ikam

# PostgreSQL backend
pip install ikam[postgres]

# Document chunking
pip install ikam[chunking]

# Full almacén support
pip install ikam[almacen]
```

## Next Steps (Day 2: FragmentCodec)

With almacén foundation complete, Day 2 focuses on:
1. **FragmentCodec:** encode/decode/validate methods
2. **Decomposition:** `decompose(doc) → List[Fragment]`
3. **Reconstruction:** `reconstruct(fragments) → Document`
4. **Round-trip validation:** `reconstruct(decompose(X)) == X`
5. **Wire to almacén:** Store fragments via CAS backend

See `docs/planning/TWO_WEEK_MVP_PLAN.md` for full schedule.

## References

- [IKAM v2 Specification](../../docs/ikam/ikam-v2-fragmented-knowledge-system.md)
- [Storage Gains Example](../../docs/ikam/STORAGE_GAINS_EXAMPLE.md)
- [Storage Observability HOWTO](../../docs/ikam/IKAM_STORAGE_OBSERVABILITY_HOWTO.md)
- [Two-Week MVP Plan](../../docs/planning/TWO_WEEK_MVP_PLAN.md)

---

**Completion Criteria Met:**
- ✅ chunking.py copied from modelado to ikam/almacen
- ✅ PostgreSQL backend implements StorageBackend with CAS
- ✅ Registry + TieredStorage stub classes exist
- ✅ Backward-compatible re-export in modelado with deprecation
- ✅ Round-trip tests written (8 tests covering CAS, lossless, dedup)

**Acceptance Test:** Run `pytest packages/ikam/tests/test_almacen_roundtrip.py` → All pass ✅
