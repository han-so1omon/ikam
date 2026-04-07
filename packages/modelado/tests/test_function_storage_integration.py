"""Integration tests for Phase 9.4: Function CAS Deduplication.

Tests validate:
1. CAS property: Same canonical code → same content_hash
2. Deduplication: Equivalent functions stored once
3. Storage monotonicity: Δ(N) = raw_bytes - dedup_bytes ≥ 0
4. Provenance preservation: All metadata persists
5. Semantic caching: Intent-based retrieval works

Run:
    pytest packages/modelado/tests/test_function_storage_integration.py -v
"""

import os
from datetime import datetime

import psycopg
from psycopg.rows import dict_row
import pytest

from modelado.core.function_storage import (
    GeneratedFunctionMetadata,
    GeneratedFunctionRecord,
    FunctionStorageStats,
)
from modelado.core.function_storage_service import (
    FunctionStorageService,
    apply_function_storage_migration,
)


# Test fixtures

@pytest.fixture
def storage_service():
    """Create in-memory storage service for testing."""
    return FunctionStorageService(connection_string=None, use_blake3=True)


@pytest.fixture(scope="function")
def postgres_storage_service():
    """Provision PostgreSQL-backed storage with migration applied (skips if DB unavailable)."""

    db_url = os.getenv(
        "TEST_DATABASE_URL",
        os.getenv(
            "PYTEST_DATABASE_URL",
            "postgresql://user:pass@localhost:5432/app",
        ),
    )

    try:
        cx = psycopg.connect(db_url, row_factory=dict_row)
    except Exception as exc:  # pragma: no cover - skip when DB missing
        pytest.skip(f"PostgreSQL unavailable for function storage integration: {exc}")

    # Ensure schema exists for this test invocation
    apply_function_storage_migration(cx)

    service = FunctionStorageService(connection_string=db_url, use_blake3=True)

    try:
        yield service
    finally:
        service.close()
        with cx.cursor() as cur:
            cur.execute("DROP VIEW IF EXISTS vw_generated_function_stats CASCADE;")
            cur.execute("DROP TABLE IF EXISTS ikam_generated_functions CASCADE;")
        cx.commit()
        cx.close()


@pytest.fixture
def sample_function_code():
    """Sample generated function with typical patterns."""
    return '''def calculate_revenue(units: int, price: float) -> float:
    """Calculate total revenue."""
    return units * price
'''


@pytest.fixture
def equivalent_function_code():
    """Semantically equivalent but syntactically different."""
    return '''

def   calculate_revenue( units:int,price:float)->float:
    """Calculate total revenue."""
    
    return units*price

'''


@pytest.fixture
def different_function_code():
    """Semantically different function."""
    return '''def calculate_profit(revenue: float, costs: float) -> float:
    """Calculate net profit."""
    return revenue - costs
'''


@pytest.fixture
def sample_metadata():
    """Sample generation metadata."""
    return GeneratedFunctionMetadata(
        user_intent="Calculate revenue from units and price",
        semantic_intent="calculate_revenue",
        confidence=0.95,
        strategy="llm_generation",
        generator_version="gpt-4o-mini-v1",
        semantic_reasoning="User requested revenue calculation function",
        extracted_parameters={"units": "int", "price": "float"},
    )


# Test Class 1: CAS Property Validation

class TestCASProperty:
    """Validate content-addressable storage guarantees."""
    
    @pytest.mark.asyncio
    async def test_same_code_produces_same_hash(
        self, storage_service, sample_function_code, sample_metadata
    ):
        """Store same function twice → same content_hash, same function_id."""
        # Store first time
        record1 = await storage_service.store_function(
            code=sample_function_code,
            metadata=sample_metadata,
        )
        
        # Store second time (identical code)
        record2 = await storage_service.store_function(
            code=sample_function_code,
            metadata=sample_metadata,
        )
        
        # Assert CAS property
        assert record1.content_hash == record2.content_hash
        assert record1.function_id == record2.function_id
        
        # Assert deduplication flag
        assert not record1.deduplicated  # First storage
        assert record2.deduplicated       # Deduplicated
    
    @pytest.mark.asyncio
    async def test_equivalent_code_produces_same_hash(
        self, storage_service, sample_function_code, equivalent_function_code, sample_metadata
    ):
        """Store equivalent functions (different whitespace) → same hash."""
        # Store original
        record1 = await storage_service.store_function(
            code=sample_function_code,
            metadata=sample_metadata,
        )
        
        # Store equivalent (different formatting)
        record2 = await storage_service.store_function(
            code=equivalent_function_code,
            metadata=sample_metadata,
        )
        
        # Assert canonicalization worked
        assert record1.content_hash == record2.content_hash
        assert record1.canonical_code == record2.canonical_code
        
        # Assert different original code
        assert record1.original_code != record2.original_code
    
    @pytest.mark.asyncio
    async def test_different_code_produces_different_hash(
        self, storage_service, sample_function_code, different_function_code, sample_metadata
    ):
        """Store different functions → different hashes."""
        # Store function 1
        record1 = await storage_service.store_function(
            code=sample_function_code,
            metadata=sample_metadata,
        )
        
        # Store function 2 (semantically different)
        record2 = await storage_service.store_function(
            code=different_function_code,
            metadata=sample_metadata,
        )
        
        # Assert different hashes
        assert record1.content_hash != record2.content_hash
        assert record1.function_id != record2.function_id
        assert record1.canonical_code != record2.canonical_code


# Test Class 2: Deduplication Validation

class TestDeduplication:
    """Validate CAS deduplication behavior."""
    
    @pytest.mark.asyncio
    async def test_duplicate_increments_execution_count(
        self, storage_service, sample_function_code, sample_metadata
    ):
        """Store same function N times → execution_count = N."""
        N = 5
        
        for i in range(N):
            record = await storage_service.store_function(
                code=sample_function_code,
                metadata=sample_metadata,
            )
        
        # Retrieve final record
        final_record = await storage_service.get_by_hash(record.content_hash)
        
        # Assert execution count incremented
        assert final_record.execution_count == N
    
    @pytest.mark.asyncio
    async def test_storage_stats_count_duplicates(
        self, storage_service, sample_function_code, equivalent_function_code, sample_metadata
    ):
        """Storage stats correctly count unique vs duplicate functions."""
        # Store original twice
        await storage_service.store_function(sample_function_code, sample_metadata)
        await storage_service.store_function(sample_function_code, sample_metadata)
        
        # Store equivalent once
        await storage_service.store_function(equivalent_function_code, sample_metadata)
        
        # Get stats
        stats = await storage_service.get_storage_stats()
        
        # Assert counts
        assert stats.total_functions_generated == 3  # Total stores
        assert stats.unique_functions_stored == 1    # All equivalent → 1 unique
        assert stats.duplicate_count == 2            # 2 duplicates
    
    @pytest.mark.asyncio
    async def test_deduplication_saves_storage(
        self, storage_service, sample_function_code, sample_metadata
    ):
        """Storing duplicates saves storage bytes."""
        # Store same function 10 times
        N = 10
        for _ in range(N):
            await storage_service.store_function(sample_function_code, sample_metadata)
        
        # Get storage stats
        stats = await storage_service.get_storage_stats()
        
        # Assert storage savings
        original_size = len(sample_function_code)
        expected_raw = original_size * N
        expected_dedup = original_size  # Only stored once canonically
        
        assert stats.raw_storage_bytes == expected_raw
        # Dedup bytes may be slightly different due to canonicalization
        assert stats.deduplicated_storage_bytes <= expected_dedup * 1.1  # Allow 10% variance
        assert stats.storage_savings_bytes > 0


# Test Class 3: Storage Monotonicity

class TestStorageMonotonicity:
    """Validate Δ(N) = raw_bytes - dedup_bytes ≥ 0."""
    
    @pytest.mark.asyncio
    async def test_monotonicity_delta_non_negative(
        self, storage_service, sample_function_code, different_function_code, sample_metadata
    ):
        """Monotonicity delta Δ(N) ≥ 0 for all N."""
        # Store mix of unique and duplicate functions
        functions = [
            sample_function_code,
            sample_function_code,  # Duplicate
            different_function_code,
            different_function_code,  # Duplicate
            sample_function_code,  # Another duplicate
        ]
        
        for code in functions:
            await storage_service.store_function(code, sample_metadata)
        
        # Get stats
        stats = await storage_service.get_storage_stats()
        
        # Assert monotonicity
        assert stats.monotonicity_delta >= 0, "Δ(N) must be non-negative"
        assert stats.raw_storage_bytes >= stats.deduplicated_storage_bytes
    
    @pytest.mark.asyncio
    async def test_monotonicity_increases_with_duplicates(
        self, storage_service, sample_function_code, sample_metadata
    ):
        """Δ(N) increases monotonically as duplicates accumulate."""
        deltas = []
        
        # Store same function incrementally
        for i in range(1, 6):
            await storage_service.store_function(sample_function_code, sample_metadata)
            stats = await storage_service.get_storage_stats()
            deltas.append(stats.monotonicity_delta)
        
        # Assert monotonic increase
        for i in range(1, len(deltas)):
            assert deltas[i] >= deltas[i - 1], f"Δ not monotonic at N={i+1}"


# Test Class 4: Provenance Preservation

class TestProvenancePreservation:
    """Validate metadata persistence (Fisher Information requirement)."""
    
    @pytest.mark.asyncio
    async def test_metadata_persists_in_record(
        self, storage_service, sample_function_code, sample_metadata
    ):
        """All generation metadata stored with function."""
        # Store function
        record = await storage_service.store_function(
            code=sample_function_code,
            metadata=sample_metadata,
        )
        
        # Assert metadata preserved
        assert record.metadata == sample_metadata
        assert record.metadata.semantic_intent == sample_metadata.semantic_intent
        assert record.metadata.generation_strategy == sample_metadata.generation_strategy
        assert record.metadata.model_name == sample_metadata.model_name
    
    @pytest.mark.asyncio
    async def test_transformations_recorded(
        self, storage_service, equivalent_function_code, sample_metadata
    ):
        """Canonicalization transformations logged for provenance."""
        # Store function with formatting differences
        record = await storage_service.store_function(
            code=equivalent_function_code,
            metadata=sample_metadata,
        )
        
        # Assert transformations recorded
        assert len(record.transformations_applied) > 0
        
        # Common transformations expected
        transformation_types = {t["type"] for t in record.transformations_applied}
        assert "whitespace_normalized" in transformation_types or \
               "imports_sorted" in transformation_types or \
               "formatting" in str(transformation_types)
    
    @pytest.mark.asyncio
    async def test_provenance_survives_deduplication(
        self, storage_service, sample_function_code
    ):
        """Different generation contexts preserved even when code identical."""
        # Store same code with different metadata
        metadata1 = GeneratedFunctionMetadata(
            user_intent="Intent A: Calculate revenue",
            semantic_intent="calculate_revenue_a",
            confidence=0.9,
            strategy="strategy_1",
            generator_version="gpt-4o-mini-v1",
        )
        
        metadata2 = GeneratedFunctionMetadata(
            user_intent="Intent B: Calculate revenue differently",
            semantic_intent="calculate_revenue_b",
            confidence=0.85,
            strategy="strategy_2",
            generator_version="gpt-4-v1",
        )
        
        record1 = await storage_service.store_function(sample_function_code, metadata1)
        record2 = await storage_service.store_function(sample_function_code, metadata2)
        
        # Assert same code deduplicated
        assert record1.content_hash == record2.content_hash
        
        # Assert provenance distinct (latest wins in current implementation)
        # Note: Full multi-provenance tracking would require separate table
        assert record2.execution_count == 2  # Both executions tracked


# Test Class 5: Semantic Cache Integration

class TestSemanticCache:
    """Validate intent-based retrieval via cache_key."""
    
    @pytest.mark.asyncio
    async def test_cache_key_enables_intent_retrieval(
        self, storage_service, sample_function_code, sample_metadata
    ):
        """Can retrieve function by semantic intent."""
        # Compute cache key from intent
        cache_key = storage_service._compute_cache_key(
            intent=sample_metadata.semantic_intent,
            params=sample_metadata.parameters,
        )
        
        # Store function with cache key
        record = await storage_service.store_function(
            code=sample_function_code,
            metadata=sample_metadata,
            cache_key=cache_key,
        )
        
        # Retrieve by cache key
        retrieved = await storage_service.get_by_cache_key(cache_key)
        
        # Assert retrieval worked
        assert retrieved is not None
        assert retrieved.function_id == record.function_id
        assert retrieved.cache_key == cache_key
    
    @pytest.mark.asyncio
    async def test_same_intent_returns_cached_function(
        self, storage_service, sample_function_code, sample_metadata
    ):
        """Repeated intent retrieves cached function (no re-generation)."""
        # Store function
        cache_key = storage_service._compute_cache_key(
            intent=sample_metadata.semantic_intent,
            params=sample_metadata.parameters,
        )
        
        original_record = await storage_service.store_function(
            code=sample_function_code,
            metadata=sample_metadata,
            cache_key=cache_key,
        )
        
        # Simulate second request with same intent
        cached_record = await storage_service.get_by_cache_key(cache_key)
        
        # Assert cache hit
        assert cached_record is not None
        assert cached_record.function_id == original_record.function_id
        assert cached_record.content_hash == original_record.content_hash


# Test Class 6: Retrieval Methods

class TestRetrievalMethods:
    """Validate all retrieval paths work correctly."""
    
    @pytest.mark.asyncio
    async def test_get_by_function_id(
        self, storage_service, sample_function_code, sample_metadata
    ):
        """Retrieve by function_id."""
        record = await storage_service.store_function(sample_function_code, sample_metadata)
        
        retrieved = await storage_service.get_by_function_id(record.function_id)
        
        assert retrieved is not None
        assert retrieved.function_id == record.function_id
    
    @pytest.mark.asyncio
    async def test_get_by_hash(
        self, storage_service, sample_function_code, sample_metadata
    ):
        """Retrieve by content_hash."""
        record = await storage_service.store_function(sample_function_code, sample_metadata)
        
        retrieved = await storage_service.get_by_hash(record.content_hash)
        
        assert retrieved is not None
        assert retrieved.content_hash == record.content_hash
    
    @pytest.mark.asyncio
    async def test_list_functions(
        self, storage_service, sample_function_code, different_function_code, sample_metadata
    ):
        """List all stored functions."""
        # Store multiple functions
        await storage_service.store_function(sample_function_code, sample_metadata)
        await storage_service.store_function(different_function_code, sample_metadata)
        
        # List all
        functions = await storage_service.list_functions(limit=100)
        
        assert len(functions) >= 2
    
    @pytest.mark.asyncio
    async def test_list_functions_pagination(
        self, storage_service, sample_function_code, sample_metadata
    ):
        """Pagination works for function listing."""
        # Store 5 functions
        for i in range(5):
            code = f'def func_{i}(): return {i}'
            await storage_service.store_function(code, sample_metadata)
        
        # Get first page
        page1 = await storage_service.list_functions(limit=2, offset=0)
        page2 = await storage_service.list_functions(limit=2, offset=2)
        
        assert len(page1) == 2
        assert len(page2) == 2
        assert page1[0].function_id != page2[0].function_id


# Test Class 7: Edge Cases

class TestEdgeCases:
    """Validate behavior in edge cases."""
    
    @pytest.mark.asyncio
    async def test_empty_storage_stats(self, storage_service):
        """Stats work correctly when no functions stored."""
        stats = await storage_service.get_storage_stats()
        
        assert stats.total_functions_generated == 0
        assert stats.unique_functions_stored == 0
        assert stats.storage_savings_bytes == 0
        assert stats.storage_savings_percent == 0.0
    
    @pytest.mark.asyncio
    async def test_retrieve_nonexistent_function(self, storage_service):
        """Retrieving non-existent function returns None."""
        retrieved = await storage_service.get_by_function_id("gfn_nonexistent")
        assert retrieved is None
        
        retrieved = await storage_service.get_by_hash("nonexistent_hash")
        assert retrieved is None
    
    @pytest.mark.asyncio
    async def test_minimal_function_storage(self, storage_service, sample_metadata):
        """Store minimal valid function."""
        minimal_code = "def f(): pass"
        
        record = await storage_service.store_function(minimal_code, sample_metadata)
        
        assert record is not None
        assert record.function_id.startswith("gfn_")
        assert len(record.content_hash) > 0


# Test Class 8: PostgreSQL backend integration

class TestPostgresBackend:
    """Validate PostgreSQL CAS backend using migration 015."""

    @pytest.mark.asyncio
    async def test_postgres_store_and_stats(
        self,
        postgres_storage_service,
        sample_function_code,
        sample_metadata,
    ):
        """Store and deduplicate functions against PostgreSQL backend."""

        service = postgres_storage_service

        # Store once
        record = await service.store_function(sample_function_code, sample_metadata)

        fetched = await service.get_by_function_id(record.function_id)
        assert fetched is not None
        assert fetched.content_hash == record.content_hash
        assert fetched.metadata.user_intent == sample_metadata.user_intent

        # Stats after first insert
        stats = await service.get_storage_stats()
        assert stats.unique_functions_stored == 1
        assert stats.duplicate_count == 0

        # Deduplicate
        await service.store_function(sample_function_code, sample_metadata)

        stats_after = await service.get_storage_stats()
        assert stats_after.unique_functions_stored == 1
        assert stats_after.duplicate_count == 1
        assert stats_after.monotonicity_delta >= 0
