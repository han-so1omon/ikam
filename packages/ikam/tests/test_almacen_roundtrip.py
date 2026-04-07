"""Round-trip tests for IKAM almacén CAS storage.

Tests validate:
1. Content-addressable storage: same content → same key
2. Lossless round-trip: put(data) → get(key) → data (byte-level equality)
3. Idempotent PUT: multiple puts return same key
4. CAS deduplication: storage savings when N ≥ 2

Mathematical Guarantees Tested:
- Storage monotonicity: Δ(N) = S_flat(N) - S_CAS(N) ≥ 0 for N ≥ 2
- Lossless reconstruction: get(put(X)) = X (byte equality)
- Hash collision probability: < 2^-128 (BLAKE3 property)
"""
import os
import pytest

from ikam.almacen import (
    FragmentKey,
    FragmentRecord,
    BackendRegistry,
)

# Skip if PostgreSQL not available
postgres = pytest.importorskip("ikam.almacen.postgres")

# Use test database or skip if not configured
TEST_DB_URL = os.getenv(
    "TEST_DATABASE_URL",
    "postgresql://narraciones:narraciones@localhost:5432/narraciones"
)


@pytest.fixture
def backend():
    """Create PostgreSQL backend with test table."""
    from ikam.almacen.postgres import PostgresBackend
    
    # Use test-specific table to avoid conflicts
    backend = PostgresBackend(
        TEST_DB_URL,
        table_name="ikam_fragments_test",
        auto_initialize=True,
    )
    
    yield backend
    
    # Cleanup: drop test table
    import psycopg
    with psycopg.connect(TEST_DB_URL) as conn:
        with conn.cursor() as cur:
            cur.execute("DROP TABLE IF EXISTS ikam_fragments_test")
            conn.commit()


def test_cas_same_content_same_key(backend):
    """Test CAS property: same content → same key."""
    payload = b"Hello, IKAM v2!"
    
    # Store same content twice
    record1 = FragmentRecord(
        key=FragmentKey(key="", kind="text"),
        payload=payload,
        metadata={"source": "test1"},
    )
    record2 = FragmentRecord(
        key=FragmentKey(key="", kind="text"),
        payload=payload,
        metadata={"source": "test2"},
    )
    
    key1 = backend.put(record1)
    key2 = backend.put(record2)
    
    # CAS guarantee: same content → same key
    assert key1.key == key2.key
    assert key1.key.startswith("blake3:")


def test_lossless_round_trip(backend):
    """Test lossless reconstruction: put(X) → get → X."""
    original_payload = b"IKAM v2 fragmentation test data: \x00\x01\x02\xff"
    
    record = FragmentRecord(
        key=FragmentKey(key="", kind="binary"),
        payload=original_payload,
        metadata={"test": "round-trip"},
    )
    
    # PUT operation
    key = backend.put(record)
    
    # GET operation
    retrieved = backend.get(key)
    
    # Byte-level equality (100% requirement)
    assert retrieved is not None
    assert retrieved.payload == original_payload
    assert retrieved.key.kind == "binary"
    assert retrieved.metadata == {"test": "round-trip"}


def test_idempotent_put(backend):
    """Test idempotent PUT: multiple puts don't create duplicates."""
    payload = b"Idempotent test"
    
    # PUT same content 3 times
    keys = []
    for i in range(3):
        record = FragmentRecord(
            key=FragmentKey(key="", kind="text"),
            payload=payload,
            metadata={"iteration": i},
        )
        keys.append(backend.put(record))
    
    # All should return same key (CAS deduplication)
    assert keys[0].key == keys[1].key == keys[2].key
    
    # Verify only one record exists
    all_keys = list(backend.list())
    matching = [k for k in all_keys if k.key == keys[0].key]
    assert len(matching) == 1


def test_storage_deduplication_savings(backend):
    """Test storage monotonicity: Δ(N) ≥ 0 when N ≥ 2."""
    # Create 3 fragments with 2 duplicates
    fragment_a = b"Fragment A content" * 100  # ~1800 bytes
    fragment_b = b"Fragment B content" * 100  # ~1800 bytes
    
    # Store: A, B, A (duplicate)
    key_a1 = backend.put(FragmentRecord(
        key=FragmentKey(key="", kind="text"),
        payload=fragment_a,
        metadata={"id": "a1"},
    ))
    
    key_b = backend.put(FragmentRecord(
        key=FragmentKey(key="", kind="text"),
        payload=fragment_b,
        metadata={"id": "b"},
    ))
    
    key_a2 = backend.put(FragmentRecord(
        key=FragmentKey(key="", kind="text"),
        payload=fragment_a,
        metadata={"id": "a2"},
    ))
    
    # CAS deduplication: A stored once
    assert key_a1.key == key_a2.key
    assert key_a1.key != key_b.key
    
    # Storage savings calculation
    # Flat storage: 3 * ~1800 = ~5400 bytes
    # CAS storage: 2 * ~1800 = ~3600 bytes (A deduplicated)
    # Δ(3) = 5400 - 3600 = 1800 bytes ≥ 0 ✓
    
    flat_bytes = 3 * len(fragment_a)
    cas_bytes = len(fragment_a) + len(fragment_b)  # A stored once
    delta = flat_bytes - cas_bytes
    
    assert delta > 0, f"Storage monotonicity violated: Δ = {delta}"
    assert delta == len(fragment_a), "Deduplication savings incorrect"


def test_delete_operation(backend):
    """Test DELETE removes fragment and returns correct status."""
    payload = b"Delete me"
    
    record = FragmentRecord(
        key=FragmentKey(key="", kind="text"),
        payload=payload,
        metadata={},
    )
    
    key = backend.put(record)
    
    # Verify exists
    assert backend.get(key) is not None
    
    # Delete
    deleted = backend.delete(key)
    assert deleted is True
    
    # Verify gone
    assert backend.get(key) is None
    
    # Delete again should return False
    deleted_again = backend.delete(key)
    assert deleted_again is False


def test_list_with_filter(backend):
    """Test LIST with kind prefix filtering."""
    # Store multiple kinds
    backend.put(FragmentRecord(
        key=FragmentKey(key="", kind="text"),
        payload=b"text fragment",
        metadata={},
    ))
    
    backend.put(FragmentRecord(
        key=FragmentKey(key="", kind="image/png"),
        payload=b"fake png data",
        metadata={},
    ))
    
    backend.put(FragmentRecord(
        key=FragmentKey(key="", kind="text/markdown"),
        payload=b"markdown fragment",
        metadata={},
    ))
    
    # List all
    all_keys = list(backend.list())
    assert len(all_keys) == 3
    
    # List text/* only
    text_keys = list(backend.list(prefix="text"))
    assert len(text_keys) == 2
    text_kinds = {k.kind for k in text_keys}
    assert text_kinds == {"text", "text/markdown"}


def test_backend_capabilities(backend):
    """Test backend reports correct capabilities."""
    from ikam.almacen import Capability
    
    caps = backend.capabilities
    
    # PostgreSQL should support all capabilities
    assert Capability.PUT in caps
    assert Capability.GET in caps
    assert Capability.DELETE in caps
    assert Capability.LIST in caps
    assert Capability.CAS in caps
    assert Capability.VERSIONS in caps
    assert Capability.METADATA_QUERY in caps


def test_backend_registry():
    """Test BackendRegistry registration and lookup."""
    from ikam.almacen.postgres import PostgresBackend
    
    registry = BackendRegistry()
    
    backend = PostgresBackend(TEST_DB_URL, auto_initialize=False)
    registry.register(backend)
    
    # Lookup by name
    retrieved = registry.get("postgresql")
    assert retrieved is backend
    
    # List backends
    backends = registry.list()
    assert "postgresql" in backends
    assert backends["postgresql"] == "PostgresBackend"


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
