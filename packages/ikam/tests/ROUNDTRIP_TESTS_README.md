"""README for IKAM graph round-trip tests.

## Test Coverage

The round-trip test suite validates IKAM's mathematical guarantees:

### 1. Lossless Reconstruction
- `test_single_fragment_roundtrip`: Single fragment → artifact → reconstruct = original bytes
- `test_multi_fragment_ordered_roundtrip`: Multiple fragments preserve order during reconstruction
- `test_empty_artifact_roundtrip`: Edge case with zero fragments
- `test_large_fragment_roundtrip`: 2MB+ fragments maintain byte equality
- `test_binary_data_roundtrip`: Non-UTF8 binary data preserved exactly

**Mathematical Guarantee:** ∀ artifact A: reconstruct(decompose(A)) = A (byte-level equality)

### 2. Storage Monotonicity
- `test_cas_deduplication_storage_savings`: Shared fragments stored once → Δ(N) > 0 for N ≥ 2
- `test_storage_monotonicity_with_variations`: Variations share base fragments → total storage < flat storage

**Mathematical Guarantee:** Δ(N) = S_flat(N) - S_IKAM(N) ≥ 0 for N outputs with shared content

### 3. Content-Addressable Storage (CAS)
- `test_identical_content_yields_same_fragment_id`: Same content → same blake3 ID
- CAS ensures idempotent storage: multiple inserts of identical content are no-ops

**Mathematical Guarantee:** Hash(X) = Hash(Y) ⟺ X = Y (collision probability < 2^-128)

## Running Tests

### Local execution (requires Postgres)
```bash
# Set PYTHONPATH to include ikam and modelado packages
export PYTHONPATH=packages/ikam/src:packages/modelado/src

# Run with test database
TEST_DATABASE_URL=postgresql://narraciones:narraciones@localhost:5432/narraciones \
  pytest packages/ikam/tests/test_graph_roundtrip.py -v

# Run integration tests
pytest packages/modelado/tests/test_graph_roundtrip_integration.py -v
```

### With IKAM test suite
```bash
# Apply migration first
python scripts/database/apply_ikam_graph_migration.py \
  --database-url postgresql://narraciones:narraciones@localhost:5432/narraciones

# Run all IKAM tests
RUN_IKAM_TESTS=1 \
  TEST_DATABASE_URL=postgresql://narraciones:narraciones@localhost:5432/narraciones \
  pytest packages/ikam/tests/ -v
```

## Test Fixtures

- `db_connection`: Provides clean DB connection with IKAM graph schema applied; truncates after each test
- `db_with_schema`: Same as above for integration tests in modelado package

## Storage Delta Calculation

The tests validate storage monotonicity by comparing:
- **S_flat(N)**: Total bytes if each artifact stored in full (no deduplication)
- **S_IKAM(N)**: Total bytes with CAS fragment storage (shared fragments counted once)
- **Δ(N)**: Storage savings = S_flat(N) - S_IKAM(N)

Expected outcomes:
- N = 1: Δ(1) ≈ 0 (no sharing)
- N ≥ 2 with shared content: Δ(N) > 0 (monotonically increasing with sharing)

## Coverage Gaps (Future Work)

- [ ] Provenance chain reconstruction (ensure event ordering)
- [ ] Delta chain reconstruction with L≤3 bound enforcement
- [ ] Variation determinism: same seed + rendererVersion → same output
- [ ] Fisher Information dominance: I_IKAM(θ) ≥ I_RAG(θ) + Δ_provenance
- [ ] Concurrent insert/retrieval stress tests (>10k fragments)
- [ ] Storage lifecycle: cold tiering and archival reconstruction
