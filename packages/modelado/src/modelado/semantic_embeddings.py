"""
Semantic Embedding & Similarity Module for Generative Operations

This module provides semantic embedding generation and similarity computation
for intent classification and semantic matching. Uses OpenAI's text-embedding-3-small
model with pgvector caching for performance.

Design Principles:
- Always use semantic matching, never hardcoded type checks
- Cache embeddings for performance (pgvector)
- Support batch operations for efficiency
- Log similarity scores for observability
"""

import asyncio
import logging
import os
from dataclasses import dataclass
from datetime import datetime
from typing import List, Optional, Dict, Any, Tuple

import blake3
import numpy as np
from modelado.oraculo.factory import create_ai_client_from_env
from modelado.oraculo.ai_client import EmbedRequest, AIClient
from modelado.oraculo.providers.openai_client import OpenAIUnifiedAIClient

logger = logging.getLogger(__name__)


@dataclass
class EmbeddingResult:
    """Result of embedding generation."""
    
    text: str
    embedding: List[float]
    model: str
    embedding_id: str  # Content hash for caching
    cached: bool
    generation_time_ms: float
    timestamp: datetime


@dataclass
class SimilarityResult:
    """Result of similarity computation."""
    
    text1: str
    text2: str
    similarity: float  # 0.0-1.0, cosine similarity
    meets_threshold: bool  # similarity >= threshold
    threshold: float
    embedding1_id: str
    embedding2_id: str
    computation_time_ms: float


class SemanticEmbeddings:
    """
    Generate and cache semantic embeddings for intent classification.
    
    Uses OpenAI text-embedding-3-small (1536 dimensions) with pgvector caching.
    Similarity computed via cosine similarity with configurable threshold.
    """
    
    def __init__(
        self,
        openai_api_key: str | None = None,
        model: str = "text-embedding-3-small",
        default_threshold: float = 0.75,
        cache_storage: Optional[Any] = None,  # PostgreSQL with pgvector
        ai_client: AIClient | None = None,
    ):
        """
        Initialize semantic embeddings engine.
        
        Args:
            openai_api_key: OpenAI API key for embedding generation
            model: Embedding model name (default: text-embedding-3-small)
            default_threshold: Default similarity threshold (default: 0.75)
            cache_storage: Optional storage adapter for embedding cache
        """
        if ai_client is not None:
            self.ai_client = ai_client
        elif openai_api_key:
            self.ai_client = OpenAIUnifiedAIClient(
                model=os.getenv("LLM_MODEL", "gpt-4o-mini"),
                embed_model=model,
                judge_model=os.getenv("LLM_JUDGE_MODEL", "gpt-4o-mini"),
                api_key=openai_api_key,
            )
        else:
            self.ai_client = create_ai_client_from_env()
        self.model = model
        self.default_threshold = default_threshold
        self.cache_storage = cache_storage
        
        # In-memory cache (LRU with max 10,000 embeddings)
        self._memory_cache: Dict[str, EmbeddingResult] = {}
        self._cache_max_size = 10_000
        
        logger.info(
            "SemanticEmbeddings initialized: model=%s, threshold=%.2f, cache=%s",
            model,
            default_threshold,
            "enabled" if cache_storage else "memory-only",
        )
    
    def _compute_embedding_id(self, text: str) -> str:
        """Compute content-addressable ID for embedding caching."""
        # Hash: text + model version
        content = f"{text}|{self.model}"
        return blake3.blake3(content.encode()).hexdigest()[:16]
    
    async def _get_cached_embedding(self, embedding_id: str) -> Optional[EmbeddingResult]:
        """Retrieve embedding from cache (memory or storage)."""
        # Check memory cache first
        if embedding_id in self._memory_cache:
            logger.debug("Embedding cache hit (memory): id=%s", embedding_id)
            return self._memory_cache[embedding_id]
        
        # Check persistent storage if available
        if self.cache_storage:
            try:
                result = await self.cache_storage.get_embedding(embedding_id)
                if result:
                    logger.debug("Embedding cache hit (storage): id=%s", embedding_id)
                    # Populate memory cache
                    self._memory_cache[embedding_id] = result
                    return result
            except Exception as exc:
                logger.warning("Embedding cache read failed: %s", exc)
        
        return None
    
    async def _store_cached_embedding(self, result: EmbeddingResult) -> None:
        """Store embedding in cache (memory and storage)."""
        # Store in memory cache (with LRU eviction)
        if len(self._memory_cache) >= self._cache_max_size:
            # Simple FIFO eviction (could use LRU with OrderedDict)
            oldest_key = next(iter(self._memory_cache))
            del self._memory_cache[oldest_key]
            logger.debug("Evicted oldest embedding from memory cache")
        
        self._memory_cache[result.embedding_id] = result
        
        # Store in persistent storage if available
        if self.cache_storage:
            try:
                await self.cache_storage.store_embedding(result)
            except Exception as exc:
                logger.warning("Embedding cache write failed: %s", exc)
    
    async def embed(self, text: str) -> EmbeddingResult:
        """
        Generate embedding for text with caching.
        
        Args:
            text: Text to embed
        
        Returns:
            EmbeddingResult with embedding vector and metadata
        """
        start_time = asyncio.get_event_loop().time()
        embedding_id = self._compute_embedding_id(text)
        
        # Check cache first
        cached = await self._get_cached_embedding(embedding_id)
        if cached:
            return cached
        
        # Generate new embedding
        try:
            response = await self.ai_client.embed(
                EmbedRequest(
                    texts=[text],
                    model=self.model,
                    metadata={"component": "SemanticEmbeddings.embed"},
                )
            )

            embedding = response.vectors[0]
            generation_time_ms = (asyncio.get_event_loop().time() - start_time) * 1000
            
            result = EmbeddingResult(
                text=text,
                embedding=embedding,
                model=self.model,
                embedding_id=embedding_id,
                cached=False,
                generation_time_ms=generation_time_ms,
                timestamp=datetime.utcnow(),
            )
            
            # Store in cache
            await self._store_cached_embedding(result)
            
            logger.info(
                "Generated embedding: id=%s, time=%.2fms",
                embedding_id,
                generation_time_ms,
            )
            
            return result
            
        except Exception as exc:
            logger.error("Embedding generation failed: text=%r, error=%s", text, exc)
            raise
    
    async def embed_batch(self, texts: List[str]) -> List[EmbeddingResult]:
        """
        Generate embeddings for multiple texts in parallel.
        
        Args:
            texts: List of texts to embed
        
        Returns:
            List of EmbeddingResults in same order as input
        """
        tasks = [self.embed(text) for text in texts]
        return await asyncio.gather(*tasks)
    
    def cosine_similarity(
        self,
        embedding1: List[float],
        embedding2: List[float],
    ) -> float:
        """
        Compute cosine similarity between two embeddings.
        
        Args:
            embedding1: First embedding vector
            embedding2: Second embedding vector
        
        Returns:
            Similarity score (0.0-1.0, higher is more similar)
        """
        vec1 = np.array(embedding1)
        vec2 = np.array(embedding2)
        
        # Cosine similarity: dot(a, b) / (norm(a) * norm(b))
        dot_product = np.dot(vec1, vec2)
        norm1 = np.linalg.norm(vec1)
        norm2 = np.linalg.norm(vec2)
        
        if norm1 == 0 or norm2 == 0:
            return 0.0
        
        similarity = dot_product / (norm1 * norm2)
        
        # Clamp to [0, 1] (cosine can be slightly negative due to float precision)
        return float(max(0.0, min(1.0, similarity)))
    
    async def compute_similarity(
        self,
        text1: str,
        text2: str,
        threshold: Optional[float] = None,
    ) -> SimilarityResult:
        """
        Compute semantic similarity between two texts.
        
        Args:
            text1: First text
            text2: Second text
            threshold: Similarity threshold (default: self.default_threshold)
        
        Returns:
            SimilarityResult with similarity score and threshold comparison
        """
        start_time = asyncio.get_event_loop().time()
        threshold = threshold if threshold is not None else self.default_threshold
        
        # Generate embeddings (uses cache)
        emb1, emb2 = await asyncio.gather(
            self.embed(text1),
            self.embed(text2),
        )
        
        # Compute similarity
        similarity = self.cosine_similarity(emb1.embedding, emb2.embedding)
        computation_time_ms = (asyncio.get_event_loop().time() - start_time) * 1000
        
        result = SimilarityResult(
            text1=text1,
            text2=text2,
            similarity=similarity,
            meets_threshold=similarity >= threshold,
            threshold=threshold,
            embedding1_id=emb1.embedding_id,
            embedding2_id=emb2.embedding_id,
            computation_time_ms=computation_time_ms,
        )
        
        logger.debug(
            "Similarity computed: %.3f (%s threshold %.2f), time=%.2fms",
            similarity,
            "meets" if result.meets_threshold else "below",
            threshold,
            computation_time_ms,
        )
        
        return result
    
    async def find_most_similar(
        self,
        query: str,
        candidates: List[str],
        threshold: Optional[float] = None,
        top_k: int = 1,
    ) -> List[Tuple[str, float]]:
        """
        Find most similar candidates to query text.
        
        Args:
            query: Query text to match
            candidates: List of candidate texts
            threshold: Minimum similarity threshold
            top_k: Return top K matches (default: 1)
        
        Returns:
            List of (candidate, similarity) tuples, sorted by similarity (descending)
        """
        threshold = threshold if threshold is not None else self.default_threshold
        
        # Generate embeddings for all texts
        query_emb = await self.embed(query)
        candidate_embs = await self.embed_batch(candidates)
        
        # Compute similarities
        similarities = [
            (candidate, self.cosine_similarity(query_emb.embedding, emb.embedding))
            for candidate, emb in zip(candidates, candidate_embs)
        ]
        
        # Filter by threshold and sort
        filtered = [(cand, sim) for cand, sim in similarities if sim >= threshold]
        sorted_results = sorted(filtered, key=lambda x: x[1], reverse=True)
        
        # Return top K
        return sorted_results[:top_k]
    
    async def clear_cache(self) -> int:
        """
        Clear embedding cache (memory only, not persistent storage).
        
        Returns:
            Number of embeddings cleared
        """
        count = len(self._memory_cache)
        self._memory_cache.clear()
        logger.info("Cleared embedding cache: %d embeddings removed", count)
        return count
    
    def get_cache_stats(self) -> Dict[str, Any]:
        """Get cache statistics."""
        return {
            "memory_cache_size": len(self._memory_cache),
            "memory_cache_max_size": self._cache_max_size,
            "persistent_cache_enabled": self.cache_storage is not None,
            "model": self.model,
            "default_threshold": self.default_threshold,
        }


class EmbeddingCacheStorage:
    """
    PostgreSQL + pgvector storage adapter for embedding cache.
    
    Schema:
        CREATE TABLE semantic_embeddings (
            id TEXT PRIMARY KEY,
            text TEXT NOT NULL,
            embedding vector(1536),
            model TEXT NOT NULL,
            created_at TIMESTAMP DEFAULT NOW()
        );
        
        CREATE INDEX ON semantic_embeddings USING ivfflat (embedding vector_cosine_ops);
    """
    
    def __init__(self, connection_string: str):
        """
        Initialize cache storage.
        
        Args:
            connection_string: PostgreSQL connection string
        """
        # Implementation deferred - will use existing database utilities
        self.connection_string = connection_string
        logger.info("EmbeddingCacheStorage initialized (schema validation pending)")
    
    async def get_embedding(self, embedding_id: str) -> Optional[EmbeddingResult]:
        """Retrieve embedding from persistent storage."""
        # Implementation deferred
        return None
    
    async def store_embedding(self, result: EmbeddingResult) -> None:
        """Store embedding in persistent storage."""
        # Implementation deferred
        pass
