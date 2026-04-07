"""
Unit tests for semantic_embeddings.py

Tests cover:
- Embedding generation with caching
- Similarity computation
- Batch operations
- Cache management
- Error handling
"""

import asyncio
from types import SimpleNamespace
from unittest.mock import AsyncMock, MagicMock, patch
import pytest
import numpy as np

from modelado.semantic_embeddings import (
    SemanticEmbeddings,
    EmbeddingResult,
    SimilarityResult,
)


class TestSemanticEmbeddings:
    """Test SemanticEmbeddings class."""
    
    @pytest.fixture
    def mock_openai_client(self):
        """Mock unified llm client for testing."""
        client = AsyncMock()

        client.embed.return_value = SimpleNamespace(vectors=[[0.1] * 1536])

        return client
    
    @pytest.fixture
    def semantic_embeddings(self, mock_openai_client):
        """SemanticEmbeddings instance with mocked client."""
        embeddings = SemanticEmbeddings(
            openai_api_key="test-key",
            model="text-embedding-3-small",
            default_threshold=0.75,
            ai_client=mock_openai_client,
        )
        return embeddings
    
    @pytest.mark.asyncio
    async def test_embed_generates_new_embedding(self, semantic_embeddings, mock_openai_client):
        """Test embedding generation for new text."""
        text = "correlate revenue with market size"
        
        result = await semantic_embeddings.embed(text)
        
        # Verify API called
        assert mock_openai_client.embed.call_count == 1
        
        # Verify result
        assert result.text == text
        assert len(result.embedding) == 1536
        assert result.model == "text-embedding-3-small"
        assert result.cached is False
        assert result.generation_time_ms > 0
        assert result.embedding_id  # Content hash exists
    
    @pytest.mark.asyncio
    async def test_embed_uses_cache_on_second_call(self, semantic_embeddings):
        """Test embedding cache hit on repeated text."""
        text = "analyze revenue trends"
        
        # First call - generates embedding
        result1 = await semantic_embeddings.embed(text)
        assert result1.cached is False
        
        # Second call - uses cache
        result2 = await semantic_embeddings.embed(text)
        assert result2.cached is False  # Initially cached, but returned from memory
        assert result2.embedding_id == result1.embedding_id
        assert result2.embedding == result1.embedding
        
        # Verify memory cache populated
        assert result1.embedding_id in semantic_embeddings._memory_cache
    
    @pytest.mark.asyncio
    async def test_embed_batch_parallel_generation(self, semantic_embeddings, mock_openai_client):
        """Test batch embedding generation."""
        texts = [
            "correlate revenue with market size",
            "analyze sensitivity of costs",
            "generate narrative for Q4 results",
        ]
        
        results = await semantic_embeddings.embed_batch(texts)
        
        # Verify all generated
        assert len(results) == 3
        assert all(r.text in texts for r in results)
        assert mock_openai_client.embed.call_count == 3
    
    def test_cosine_similarity_identical_vectors(self, semantic_embeddings):
        """Test cosine similarity for identical vectors."""
        vec = [0.5] * 1536
        
        similarity = semantic_embeddings.cosine_similarity(vec, vec)
        
        # Identical vectors should have similarity 1.0
        assert similarity == pytest.approx(1.0, abs=1e-6)
    
    def test_cosine_similarity_orthogonal_vectors(self, semantic_embeddings):
        """Test cosine similarity for orthogonal vectors."""
        vec1 = [1.0, 0.0] + [0.0] * 1534
        vec2 = [0.0, 1.0] + [0.0] * 1534
        
        similarity = semantic_embeddings.cosine_similarity(vec1, vec2)
        
        # Orthogonal vectors should have similarity 0.0
        assert similarity == pytest.approx(0.0, abs=1e-6)
    
    def test_cosine_similarity_opposite_vectors(self, semantic_embeddings):
        """Test cosine similarity for opposite vectors."""
        vec1 = [1.0] * 1536
        vec2 = [-1.0] * 1536
        
        similarity = semantic_embeddings.cosine_similarity(vec1, vec2)
        
        # Opposite vectors should have similarity 0.0 (clamped)
        assert similarity == 0.0
    
    def test_cosine_similarity_zero_vector_handling(self, semantic_embeddings):
        """Test cosine similarity with zero vector."""
        vec1 = [0.0] * 1536
        vec2 = [1.0] * 1536
        
        similarity = semantic_embeddings.cosine_similarity(vec1, vec2)
        
        # Zero vector should return 0.0
        assert similarity == 0.0
    
    @pytest.mark.asyncio
    async def test_compute_similarity_with_threshold(self, semantic_embeddings):
        """Test similarity computation with threshold comparison."""
        text1 = "correlate revenue with market size"
        text2 = "analyze correlation between sales and market"
        
        result = await semantic_embeddings.compute_similarity(text1, text2, threshold=0.5)
        
        # Verify result structure
        assert result.text1 == text1
        assert result.text2 == text2
        assert 0.0 <= result.similarity <= 1.0
        assert result.threshold == 0.5
        assert isinstance(result.meets_threshold, bool)
        assert result.computation_time_ms > 0
    
    @pytest.mark.asyncio
    async def test_compute_similarity_uses_default_threshold(self, semantic_embeddings):
        """Test similarity computation with default threshold."""
        text1 = "revenue analysis"
        text2 = "cost analysis"
        
        result = await semantic_embeddings.compute_similarity(text1, text2)
        
        # Should use default threshold (0.75)
        assert result.threshold == 0.75
    
    @pytest.mark.asyncio
    async def test_find_most_similar_returns_top_match(self, semantic_embeddings):
        """Test finding most similar candidate."""
        query = "correlate revenue with market size"
        candidates = [
            "analyze revenue trends over time",
            "correlate sales with customer acquisition",
            "generate quarterly financial report",
        ]
        
        # Mock embeddings to ensure predictable similarity
        with patch.object(semantic_embeddings, 'embed') as mock_embed:
            # Query embedding
            query_emb = EmbeddingResult(
                text=query,
                embedding=[1.0] + [0.0] * 1535,
                model="test",
                embedding_id="query",
                cached=False,
                generation_time_ms=10.0,
                timestamp=None,
            )
            
            # Candidate embeddings (varying similarity)
            candidate_embs = [
                EmbeddingResult(
                    text=candidates[0],
                    embedding=[0.5] + [0.0] * 1535,  # Lower similarity
                    model="test",
                    embedding_id="c0",
                    cached=False,
                    generation_time_ms=10.0,
                    timestamp=None,
                ),
                EmbeddingResult(
                    text=candidates[1],
                    embedding=[0.9] + [0.1] * 1535,  # Higher similarity
                    model="test",
                    embedding_id="c1",
                    cached=False,
                    generation_time_ms=10.0,
                    timestamp=None,
                ),
                EmbeddingResult(
                    text=candidates[2],
                    embedding=[0.0, 1.0] + [0.0] * 1534,  # Orthogonal (low similarity)
                    model="test",
                    embedding_id="c2",
                    cached=False,
                    generation_time_ms=10.0,
                    timestamp=None,
                ),
            ]
            
            mock_embed.side_effect = [query_emb] + candidate_embs
            
            results = await semantic_embeddings.find_most_similar(
                query,
                candidates,
                threshold=0.3,
                top_k=2,
            )
        
        # Should return top 2 matches above threshold
        assert len(results) <= 2
        assert all(sim >= 0.3 for _, sim in results)
        
        # Should be sorted by similarity (descending)
        if len(results) > 1:
            assert results[0][1] >= results[1][1]
    
    @pytest.mark.asyncio
    async def test_find_most_similar_filters_below_threshold(self, semantic_embeddings):
        """Test that find_most_similar filters candidates below threshold."""
        query = "revenue analysis"
        candidates = ["completely unrelated text"]
        
        # Mock very low similarity
        with patch.object(semantic_embeddings, 'cosine_similarity', return_value=0.1):
            results = await semantic_embeddings.find_most_similar(
                query,
                candidates,
                threshold=0.75,  # High threshold
                top_k=5,
            )
        
        # Should return empty list (no matches above threshold)
        assert len(results) == 0
    
    @pytest.mark.asyncio
    async def test_cache_eviction_on_overflow(self, semantic_embeddings):
        """Test cache eviction when max size exceeded."""
        # Set small cache size for testing
        semantic_embeddings._cache_max_size = 3
        
        # Generate 4 embeddings (should evict oldest)
        texts = [f"text {i}" for i in range(4)]
        for text in texts:
            await semantic_embeddings.embed(text)
        
        # Cache should have max 3 items
        assert len(semantic_embeddings._memory_cache) == 3
        
        # First embedding should be evicted (FIFO)
        first_id = semantic_embeddings._compute_embedding_id(texts[0])
        assert first_id not in semantic_embeddings._memory_cache
    
    @pytest.mark.asyncio
    async def test_clear_cache_removes_all_embeddings(self, semantic_embeddings):
        """Test cache clearing."""
        # Populate cache
        texts = ["text 1", "text 2", "text 3"]
        for text in texts:
            await semantic_embeddings.embed(text)
        
        assert len(semantic_embeddings._memory_cache) > 0
        
        # Clear cache
        count = await semantic_embeddings.clear_cache()
        
        assert count == 3
        assert len(semantic_embeddings._memory_cache) == 0
    
    def test_get_cache_stats(self, semantic_embeddings):
        """Test cache statistics retrieval."""
        stats = semantic_embeddings.get_cache_stats()
        
        assert "memory_cache_size" in stats
        assert "memory_cache_max_size" in stats
        assert "persistent_cache_enabled" in stats
        assert "model" in stats
        assert "default_threshold" in stats
        
        assert stats["model"] == "text-embedding-3-small"
        assert stats["default_threshold"] == 0.75
        assert stats["persistent_cache_enabled"] is False
    
    @pytest.mark.asyncio
    async def test_embedding_error_handling(self, semantic_embeddings, mock_openai_client):
        """Test error handling during embedding generation."""
        mock_openai_client.embed.side_effect = Exception("API error")
        
        with pytest.raises(Exception, match="API error"):
            await semantic_embeddings.embed("test text")
    
    def test_compute_embedding_id_deterministic(self, semantic_embeddings):
        """Test embedding ID computation is deterministic."""
        text = "test text"
        
        id1 = semantic_embeddings._compute_embedding_id(text)
        id2 = semantic_embeddings._compute_embedding_id(text)
        
        # Same text should produce same ID
        assert id1 == id2
    
    def test_compute_embedding_id_different_for_different_text(self, semantic_embeddings):
        """Test embedding ID differs for different text."""
        text1 = "test text 1"
        text2 = "test text 2"
        
        id1 = semantic_embeddings._compute_embedding_id(text1)
        id2 = semantic_embeddings._compute_embedding_id(text2)
        
        # Different text should produce different IDs
        assert id1 != id2


class TestEmbeddingResult:
    """Test EmbeddingResult dataclass."""
    
    def test_embedding_result_creation(self):
        """Test creating EmbeddingResult."""
        from datetime import datetime
        
        result = EmbeddingResult(
            text="test text",
            embedding=[0.1, 0.2, 0.3],
            model="text-embedding-3-small",
            embedding_id="abc123",
            cached=False,
            generation_time_ms=15.5,
            timestamp=datetime.utcnow(),
        )
        
        assert result.text == "test text"
        assert result.embedding == [0.1, 0.2, 0.3]
        assert result.model == "text-embedding-3-small"
        assert result.embedding_id == "abc123"
        assert result.cached is False
        assert result.generation_time_ms == 15.5


class TestSimilarityResult:
    """Test SimilarityResult dataclass."""
    
    def test_similarity_result_creation(self):
        """Test creating SimilarityResult."""
        result = SimilarityResult(
            text1="text 1",
            text2="text 2",
            similarity=0.85,
            meets_threshold=True,
            threshold=0.75,
            embedding1_id="id1",
            embedding2_id="id2",
            computation_time_ms=20.0,
        )
        
        assert result.text1 == "text 1"
        assert result.text2 == "text 2"
        assert result.similarity == 0.85
        assert result.meets_threshold is True
        assert result.threshold == 0.75
        assert result.computation_time_ms == 20.0
