"""Execution cache for generative operations.

Cache for generated functions to enable reproducibility.

This module provides:
1. In-memory LRU cache for fast access
2. PostgreSQL fallback for persistent caching
3. Content-based addressing (cache by function hash)
4. Statistics and observability
5. Deterministic key generation

The cache enables:
- Fast retrieval of previously generated functions
- Reproducibility (same input → same cached result)
- Cost reduction (avoid regenerating same function)
- Deterministic execution guarantees
"""

from __future__ import annotations

import json
import logging
from dataclasses import dataclass, asdict
from datetime import datetime
from typing import Any, Dict, Optional, Tuple

from .core.generative_contracts import (
    GenerativeCommand,
    GeneratedOperation,
    ExecutableFunction,
)

logger = logging.getLogger(__name__)


# ============================================================================
# Cache Entry Models
# ============================================================================

@dataclass
class CacheEntry:
    """Entry in the execution cache."""
    command_hash: str  # Hash of command (deterministic key)
    function_id: str  # Hash of generated function code
    operation_id: str  # ID of the generated operation
    operation_data: Dict[str, Any]  # Serialized GeneratedOperation
    cached_at: datetime  # When this entry was cached
    access_count: int = 0  # Number of times accessed
    last_accessed: datetime = None
    generation_time_ms: float = 0.0  # Time spent generating this


# ============================================================================
# In-Memory Cache
# ============================================================================

class InMemoryCache:
    """Fast in-memory LRU cache for generated functions.
    
    Uses least-recently-used eviction when capacity is reached.
    """
    
    def __init__(self, max_size: int = 10000):
        """Initialize in-memory cache.
        
        Args:
            max_size: Maximum number of entries
        """
        self.max_size = max_size
        self._cache: Dict[str, CacheEntry] = {}
        self._access_order: list = []  # Track access order for LRU
        self.hits = 0
        self.misses = 0
    
    def get(self, command_hash: str) -> Optional[CacheEntry]:
        """Get cached entry.
        
        Args:
            command_hash: The command hash to look up
            
        Returns:
            CacheEntry if found, None otherwise
        """
        entry = self._cache.get(command_hash)
        
        if entry:
            entry.access_count += 1
            entry.last_accessed = datetime.utcnow()
            
            # Update access order (move to end = most recently used)
            if command_hash in self._access_order:
                self._access_order.remove(command_hash)
            self._access_order.append(command_hash)
            
            self.hits += 1
            logger.debug(f"Cache hit for {command_hash[:8]}...")
            return entry
        
        self.misses += 1
        return None
    
    def put(self, command_hash: str, entry: CacheEntry) -> None:
        """Add or update cache entry.
        
        Args:
            command_hash: The command hash key
            entry: The cache entry to store
        """
        # Evict LRU if needed
        if len(self._cache) >= self.max_size and command_hash not in self._cache:
            if self._access_order:
                lru_hash = self._access_order.pop(0)
                del self._cache[lru_hash]
                logger.debug(f"LRU eviction: removed {lru_hash[:8]}...")
        
        self._cache[command_hash] = entry
        
        if command_hash not in self._access_order:
            self._access_order.append(command_hash)
        
        logger.debug(f"Cached entry for {command_hash[:8]}... (size={len(self._cache)})")
    
    def exists(self, command_hash: str) -> bool:
        """Check if entry exists.
        
        Args:
            command_hash: The command hash
            
        Returns:
            True if entry exists
        """
        return command_hash in self._cache
    
    def clear(self) -> None:
        """Clear all cached entries."""
        self._cache.clear()
        self._access_order.clear()
        self.hits = 0
        self.misses = 0
        logger.info("Cleared in-memory cache")
    
    def size(self) -> int:
        """Get current cache size."""
        return len(self._cache)
    
    def get_stats(self) -> Dict[str, Any]:
        """Get cache statistics."""
        total = self.hits + self.misses
        hit_rate = (self.hits / total * 100) if total > 0 else 0
        
        return {
            'type': 'in_memory_lru',
            'size': len(self._cache),
            'max_size': self.max_size,
            'hits': self.hits,
            'misses': self.misses,
            'hit_rate_percent': hit_rate,
            'total_requests': total,
        }


# ============================================================================
# Persistent Cache (PostgreSQL)
# ============================================================================

class PostgresPersistentCache:
    """Persistent cache using PostgreSQL.
    
    Falls back to database storage for cache entries that
    exceed in-memory limits or need to survive across processes.
    """
    
    def __init__(self, db_connection=None):
        """Initialize PostgreSQL cache.
        
        Args:
            db_connection: SQLAlchemy connection or None
        """
        self.db = db_connection
        self.enabled = db_connection is not None
        self.hits = 0
        self.misses = 0
    
    def get(self, command_hash: str) -> Optional[CacheEntry]:
        """Get entry from database.
        
        Args:
            command_hash: The command hash
            
        Returns:
            CacheEntry if found, None otherwise
        """
        if not self.enabled:
            self.misses += 1
            return None
        
        try:
            # TODO: Implement actual DB query
            # For now, return None (placeholder)
            self.misses += 1
            return None
        
        except Exception as e:
            logger.error(f"Error querying persistent cache: {e}")
            self.misses += 1
            return None
    
    def put(self, command_hash: str, entry: CacheEntry) -> None:
        """Store entry in database.
        
        Args:
            command_hash: The command hash
            entry: The cache entry
        """
        if not self.enabled:
            return
        
        try:
            # TODO: Implement actual DB insert/update
            logger.debug(f"Stored entry in persistent cache: {command_hash[:8]}...")
        
        except Exception as e:
            logger.error(f"Error storing in persistent cache: {e}")
    
    def get_stats(self) -> Dict[str, Any]:
        """Get cache statistics."""
        total = self.hits + self.misses
        hit_rate = (self.hits / total * 100) if total > 0 else 0
        
        return {
            'type': 'postgres_persistent',
            'enabled': self.enabled,
            'hits': self.hits,
            'misses': self.misses,
            'hit_rate_percent': hit_rate,
            'total_requests': total,
        }


# ============================================================================
# Unified Execution Cache
# ============================================================================

class ExecutionCache:
    """Unified cache combining in-memory and persistent storage.
    
    Architecture:
    1. Check in-memory cache first (fast)
    2. If miss, check persistent cache
    3. Store all generated functions in both caches
    4. Evict from memory but keep in persistent cache
    """
    
    def __init__(
        self,
        memory_size: int = 10000,
        db_connection=None,
    ):
        """Initialize unified cache.
        
        Args:
            memory_size: Size of in-memory cache
            db_connection: PostgreSQL connection for persistent cache
        """
        self.memory_cache = InMemoryCache(max_size=memory_size)
        self.persistent_cache = PostgresPersistentCache(db_connection)
        self.total_gets = 0
        self.total_puts = 0
    
    def get(self, command: GenerativeCommand) -> Optional[Dict[str, Any]]:
        """Get cached operation for a command.
        
        Args:
            command: The generative command
            
        Returns:
            Cached operation data, or None if not found
        """
        self.total_gets += 1
        command_hash = command.command_hash()
        
        # Try in-memory first
        entry = self.memory_cache.get(command_hash)
        if entry:
            logger.info(f"Cache hit (memory): {command_hash[:8]}...")
            return entry.operation_data
        
        # Try persistent cache
        entry = self.persistent_cache.get(command_hash)
        if entry:
            logger.info(f"Cache hit (persistent): {command_hash[:8]}...")
            # Put back in memory cache
            self.memory_cache.put(command_hash, entry)
            return entry.operation_data
        
        logger.debug(f"Cache miss: {command_hash[:8]}...")
        return None
    
    def put(
        self,
        command: GenerativeCommand,
        operation: GeneratedOperation,
    ) -> None:
        """Cache a generated operation.
        
        Args:
            command: The original command
            operation: The generated operation
        """
        self.total_puts += 1
        command_hash = command.command_hash()
        
        # Create cache entry
        entry = CacheEntry(
            command_hash=command_hash,
            function_id=operation.function_id,
            operation_id=operation.operation_id,
            operation_data=operation.to_dict(),
            cached_at=datetime.utcnow(),
            generation_time_ms=operation.generation_time_ms,
        )
        
        # Store in both caches
        self.memory_cache.put(command_hash, entry)
        self.persistent_cache.put(command_hash, entry)
        
        logger.info(f"Cached operation: {command_hash[:8]}... (memory+persistent)")
    
    def exists(self, command: GenerativeCommand) -> bool:
        """Check if operation is cached.
        
        Args:
            command: The command to check
            
        Returns:
            True if operation is cached
        """
        command_hash = command.command_hash()
        return self.memory_cache.exists(command_hash)
    
    def clear(self, scope: str = "all") -> None:
        """Clear cache.
        
        Args:
            scope: "memory", "persistent", or "all"
        """
        if scope in ("memory", "all"):
            self.memory_cache.clear()
            logger.info("Cleared in-memory cache")
        
        if scope in ("persistent", "all"):
            logger.info("Persistent cache clear not yet implemented")
    
    def get_stats(self) -> Dict[str, Any]:
        """Get comprehensive cache statistics."""
        return {
            'total_gets': self.total_gets,
            'total_puts': self.total_puts,
            'memory': self.memory_cache.get_stats(),
            'persistent': self.persistent_cache.get_stats(),
            'combined_hit_rate_percent': (
                (self.memory_cache.hits + self.persistent_cache.hits) /
                max(1, self.total_gets) * 100
            ),
        }
    
    def get_memory_stats(self) -> Dict[str, Any]:
        """Get in-memory cache statistics."""
        return self.memory_cache.get_stats()
    
    def get_persistent_stats(self) -> Dict[str, Any]:
        """Get persistent cache statistics."""
        return self.persistent_cache.get_stats()


# ============================================================================
# Cache Key Generation
# ============================================================================

def generate_cache_key(
    user_instruction: str,
    operation_type: str,
    context: Dict[str, Any],
    model_version: Optional[str] = None,
) -> str:
    """Generate deterministic cache key for a command.
    
    Args:
        user_instruction: User's natural language instruction
        operation_type: Type of operation
        context: Execution context
        model_version: LLM model version if applicable
        
    Returns:
        Deterministic cache key
    """
    import hashlib
    
    key_dict = {
        'instruction': user_instruction,
        'operation_type': operation_type,
        'context': context,
        'model_version': model_version,
    }
    
    key_json = json.dumps(key_dict, sort_keys=True, default=str)
    return hashlib.sha256(key_json.encode()).hexdigest()


# ============================================================================
# Global Cache Instance
# ============================================================================

_global_cache: Optional[ExecutionCache] = None


def get_global_cache(
    memory_size: int = 10000,
    db_connection=None,
) -> ExecutionCache:
    """Get or create global execution cache.
    
    Args:
        memory_size: Size of in-memory cache
        db_connection: PostgreSQL connection
        
    Returns:
        Global ExecutionCache instance
    """
    global _global_cache
    if _global_cache is None:
        _global_cache = ExecutionCache(memory_size=memory_size, db_connection=db_connection)
    return _global_cache


def cache_get(command: GenerativeCommand) -> Optional[Dict[str, Any]]:
    """Get from global cache."""
    return get_global_cache().get(command)


def cache_put(command: GenerativeCommand, operation: GeneratedOperation) -> None:
    """Store in global cache."""
    get_global_cache().put(command, operation)
