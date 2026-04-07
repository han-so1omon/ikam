"""Unified command router for generative operations.

Routes GenerativeCommand to appropriate handler.

This module implements the core routing logic for completely generative operations:
1. Accepts GenerativeCommand input
2. Routes to appropriate handler based on operation_type
3. Manages execution cache to avoid regenerating same function
4. Coordinates with downstream processors (semantic engine, generators)

The router is the central nervous system of the generative operations system.
"""

from __future__ import annotations

import asyncio
import logging
from dataclasses import dataclass
from datetime import datetime
from typing import Any, Dict, List, Optional
from uuid import uuid4

from .generative_contracts import (
    GenerativeCommand,
    GeneratedOperation,
    ExecutableFunction,
    GenerationStrategy,
    ConstraintType,
)
from .deliberation_sink import (
    DeliberationSink,
    NoOpDeliberationSink,
    DeliberationContext,
    DeliberationEnvelopePayload,
)

logger = logging.getLogger(__name__)


# ============================================================================
# Handler Interface and Registry
# ============================================================================

class GenerativeHandler:
    """Base class for handlers that process GenerativeCommand."""
    
    def __init__(self, name: str, operation_type: str):
        """Initialize handler.
        
        Args:
            name: Human-readable handler name
            operation_type: The operation_type this handler processes
        """
        self.name = name
        self.operation_type = operation_type
        self.processed_count = 0
        self.error_count = 0
        self.cache_hits = 0
        self.total_generation_time_ms = 0.0
    
    async def handle(self, command: GenerativeCommand) -> GeneratedOperation:
        """Process command and return generated operation.
        
        Args:
            command: The command to process
            
        Returns:
            GeneratedOperation with generated function
            
        Raises:
            ValueError: If command is invalid for this handler
            RuntimeError: If generation fails
        """
        raise NotImplementedError
    
    def can_handle(self, command: GenerativeCommand) -> bool:
        """Check if this handler can process the command.
        
        Args:
            command: The command to check
            
        Returns:
            True if this handler can process this command
        """
        return command.operation_type == self.operation_type
    
    def get_stats(self) -> Dict[str, Any]:
        """Get handler statistics."""
        return {
            'name': self.name,
            'operation_type': self.operation_type,
            'processed_count': self.processed_count,
            'error_count': self.error_count,
            'cache_hits': self.cache_hits,
            'avg_generation_time_ms': (
                self.total_generation_time_ms / max(1, self.processed_count)
            ),
        }


@dataclass
class HandlerRegistration:
    """Registration record for a handler."""
    handler: GenerativeHandler
    registered_at: datetime
    priority: int  # Lower = higher priority


class HandlerRegistry:
    """Registry for generative command handlers."""
    
    def __init__(self):
        """Initialize empty registry."""
        self._handlers: Dict[str, List[HandlerRegistration]] = {}
    
    def register(
        self,
        handler: GenerativeHandler,
        operation_type: Optional[str] = None,
        priority: int = 100,
    ) -> None:
        """Register a handler.
        
        Args:
            handler: The handler to register
            operation_type: Operation type to handle (defaults to handler.operation_type)
            priority: Priority for this handler (lower = higher priority)
        """
        op_type = operation_type or handler.operation_type
        if op_type not in self._handlers:
            self._handlers[op_type] = []
        
        registration = HandlerRegistration(
            handler=handler,
            registered_at=datetime.utcnow(),
            priority=priority,
        )
        self._handlers[op_type].append(registration)
        
        # Sort by priority
        self._handlers[op_type].sort(key=lambda r: r.priority)
        
        logger.info(
            f"Registered handler {handler.name} for {op_type} with priority {priority}"
        )
    
    def get_handler(self, operation_type: str) -> Optional[GenerativeHandler]:
        """Get the highest-priority handler for an operation type.
        
        Args:
            operation_type: The operation type to find handler for
            
        Returns:
            The handler, or None if no handler registered
        """
        registrations = self._handlers.get(operation_type, [])
        if registrations:
            return registrations[0].handler
        return None
    
    def get_all_handlers(self, operation_type: str) -> List[GenerativeHandler]:
        """Get all handlers for an operation type, sorted by priority.
        
        Args:
            operation_type: The operation type
            
        Returns:
            List of handlers, sorted by priority
        """
        registrations = self._handlers.get(operation_type, [])
        return [r.handler for r in registrations]
    
    def list_handlers(self) -> Dict[str, List[str]]:
        """List all registered handlers."""
        result = {}
        for op_type, registrations in self._handlers.items():
            result[op_type] = [r.handler.name for r in registrations]
        return result
    
    def get_stats(self) -> Dict[str, Any]:
        """Get statistics for all handlers."""
        stats = {}
        for op_type, registrations in self._handlers.items():
            stats[op_type] = [r.handler.get_stats() for r in registrations]
        return stats


# ============================================================================
# Execution Cache
# ============================================================================

@dataclass
class CacheEntry:
    """Entry in the execution cache."""
    command_hash: str
    operation: GeneratedOperation
    cached_at: datetime
    access_count: int = 0
    last_accessed: datetime = None


class ExecutionCache:
    """In-memory execution cache for generated operations.
    
    Cache is keyed by command hash (deterministic function of command parameters).
    Provides cache hits to avoid regenerating same function.
    """
    
    def __init__(self, max_size: int = 10000):
        """Initialize cache.
        
        Args:
            max_size: Maximum number of entries in cache
        """
        self.max_size = max_size
        self._cache: Dict[str, CacheEntry] = {}
        self._access_order: List[str] = []
        self.hits = 0
        self.misses = 0
    
    def get(self, command: GenerativeCommand) -> Optional[GeneratedOperation]:
        """Try to get cached operation for command.
        
        Args:
            command: The command to look up
            
        Returns:
            Cached GeneratedOperation, or None if not in cache
        """
        command_hash = command.command_hash()
        entry = self._cache.get(command_hash)
        
        if entry:
            entry.access_count += 1
            entry.last_accessed = datetime.utcnow()
            # Move to end (most recently used)
            if command_hash in self._access_order:
                self._access_order.remove(command_hash)
            self._access_order.append(command_hash)
            self.hits += 1
            logger.debug(f"Cache hit for command {command_hash}")
            return entry.operation
        
        self.misses += 1
        return None
    
    def put(self, command: GenerativeCommand, operation: GeneratedOperation) -> None:
        """Cache an operation for a command.
        
        Args:
            command: The command
            operation: The generated operation result
        """
        command_hash = command.command_hash()
        
        # If cache is full, evict least recently used
        if len(self._cache) >= self.max_size and command_hash not in self._cache:
            if self._access_order:
                lru_hash = self._access_order.pop(0)
                del self._cache[lru_hash]
                logger.debug(f"Evicted LRU entry {lru_hash} from cache")
        
        entry = CacheEntry(
            command_hash=command_hash,
            operation=operation,
            cached_at=datetime.utcnow(),
        )
        
        self._cache[command_hash] = entry
        if command_hash not in self._access_order:
            self._access_order.append(command_hash)
        
        logger.debug(f"Cached operation for command {command_hash}")
    
    def clear(self) -> None:
        """Clear entire cache."""
        self._cache.clear()
        self._access_order.clear()
        self.hits = 0
        self.misses = 0
        logger.info("Cleared execution cache")
    
    def get_stats(self) -> Dict[str, Any]:
        """Get cache statistics."""
        total = self.hits + self.misses
        hit_rate = (self.hits / total * 100) if total > 0 else 0
        
        return {
            'size': len(self._cache),
            'max_size': self.max_size,
            'hits': self.hits,
            'misses': self.misses,
            'hit_rate_percent': hit_rate,
            'total_requests': total,
        }


# ============================================================================
# Main Router
# ============================================================================

class GenerativeCommandRouter:
    """Routes GenerativeCommand to appropriate handler.
    
    This is the central dispatcher for the generative operations system.
    It handles:
    1. Cache lookup (avoid regenerating same function)
    2. Handler selection (route to correct processor)
    3. Operation execution
    4. Logging and observability
    """
    
    def __init__(
        self,
        handlers: Optional[List[GenerativeHandler]] = None,
        cache_size: int = 10000,
        handler_registry: Optional[HandlerRegistry] = None,
        execution_cache: Optional[Any] = None,
        deliberation_sink: Optional[DeliberationSink] = None,
    ):
        """Initialize router.
        
        Args:
            handlers: Initial list of handlers to register
            cache_size: Size of execution cache
        """
        # Back-compat: early integration tests inject a handler registry and a
        # unified execution cache from `modelado.execution_cache`.
        self.registry = handler_registry or HandlerRegistry()
        self.cache = execution_cache or ExecutionCache(max_size=cache_size)
        self.deliberation_sink = deliberation_sink or NoOpDeliberationSink()
        
        # Register handlers
        if handlers:
            for handler in handlers:
                self.registry.register(handler)
        
        self.total_commands_processed = 0
        self.total_errors = 0

    def _operation_from_cached_dict(self, data: Dict[str, Any]) -> GeneratedOperation:
        """Rehydrate a GeneratedOperation from a cached dict payload.

        The legacy unified cache stores `GeneratedOperation.to_dict()`.
        """
        func_data = data.get('generated_function') or {}

        constraints_enforced = []
        for raw in func_data.get('constraints_enforced') or []:
            try:
                constraints_enforced.append(ConstraintType(raw))
            except Exception:
                continue

        try:
            generation_strategy = GenerationStrategy(func_data.get('generation_strategy'))
        except Exception:
            generation_strategy = GenerationStrategy.UNKNOWN

        func = ExecutableFunction(
            name=func_data.get('name', ''),
            language=func_data.get('language', 'python'),
            code=func_data.get('code', ''),
            signature=func_data.get('signature', {}),
            constraints_enforced=constraints_enforced,
            generation_strategy=generation_strategy,
            strategy_metadata=func_data.get('strategy_metadata', {}) or {},
            function_id=func_data.get('function_id', '') or '',
            semantic_engine_version=func_data.get('semantic_engine_version', 'unknown') or 'unknown',
            model_version=func_data.get('model_version'),
            seed=func_data.get('seed'),
        )

        return GeneratedOperation(
            operation_id=data.get('operation_id', str(uuid4())),
            command_id=data.get('command_id', ''),
            generated_function=func,
            generation_metadata=data.get('generation_metadata', {}) or {},
            function_id=data.get('function_id', '') or func.function_id,
            is_cached=True,
            generation_time_ms=float(data.get('generation_time_ms') or 0.0),
            semantic_confidence=float(data.get('semantic_confidence') or 1.0),
            semantic_reasoning=data.get('semantic_reasoning'),
            selected_evaluator=data.get('selected_evaluator'),
        )
    
    def register_handler(
        self,
        handler: GenerativeHandler,
        priority: int = 100,
    ) -> None:
        """Register a handler for an operation type.
        
        Args:
            handler: The handler to register
            priority: Priority for this handler (lower = higher priority)
        """
        self.registry.register(handler, priority=priority)
    
    async def route(self, command: GenerativeCommand) -> GeneratedOperation:
        """Route and process a generative command.
        
        Process:
        1. Check cache for existing operation
        2. If cache hit, return cached operation
        3. Find appropriate handler
        4. Call handler to generate operation
        5. Cache result
        6. Return operation
        
        Args:
            command: The command to process
            
        Returns:
            GeneratedOperation with the generated function
            
        Raises:
            ValueError: If no handler found for operation_type
            RuntimeError: If handler fails
        """
        self.total_commands_processed += 1
        start_time = datetime.utcnow()
        
        try:
            # Check cache first
            cached_op = self.cache.get(command)
            if cached_op:
                logger.info(
                    f"Cache hit for command {command.command_id} "
                    f"(operation_type={command.operation_type})"
                )
                if isinstance(cached_op, GeneratedOperation):
                    return cached_op
                if isinstance(cached_op, dict):
                    return self._operation_from_cached_dict(cached_op)
                # Unknown cache payload; treat as miss.
            
            # Find appropriate handler
            handler = self.registry.get_handler(command.operation_type)
            if not handler:
                raise ValueError(
                    f"No handler found (No handler registered) for operation_type={command.operation_type}. "
                    f"Available: {list(self.registry._handlers.keys())}"
                )
            
            # Call handler to generate operation
            logger.info(
                f"Routing command {command.command_id} to handler {handler.name}"
            )
            self._emit_routing_summary(command, handler)
            operation = await handler.handle(command)
            
            # Mark as not cached (fresh generation)
            operation.is_cached = False
            
            # Cache for future use
            self.cache.put(command, operation)
            
            # Update handler stats
            elapsed_ms = (datetime.utcnow() - start_time).total_seconds() * 1000
            handler.processed_count += 1
            handler.total_generation_time_ms += elapsed_ms
            operation.generation_time_ms = elapsed_ms
            
            logger.info(
                f"Generated operation {operation.operation_id} for command {command.command_id} "
                f"in {elapsed_ms:.1f}ms"
            )
            
            return operation
            
        except Exception as e:
            self.total_errors += 1
            logger.error(f"Error processing command {command.command_id}: {e}")
            raise

    def _emit_routing_summary(
        self,
        command: GenerativeCommand,
        handler: GenerativeHandler,
    ) -> None:
        """Emit a deliberation breadcrumb summarizing routing decisions."""
        try:
            context = command.context or {}
            project_id = context.get("project_id") or context.get("projectId")
            if not project_id:
                return
            session_id = context.get("session_id") or context.get("sessionId")
            parent_id = context.get("parent_id") or context.get("parentId") or context.get("interaction_id")
            ts = int(datetime.utcnow().timestamp() * 1000)
            envelope = DeliberationEnvelopePayload(
                run_id=str(command.command_id),
                project_id=str(project_id),
                ts=ts,
                phase="decide",
                status="complete",
                summary=f"Routed {command.command_id} to {handler.name}",
            )
            ctx = DeliberationContext(
                project_id=str(project_id),
                session_id=str(session_id) if session_id is not None else None,
                parent_id=str(parent_id) if parent_id is not None else None,
                run_id=str(command.command_id),
            )
            self.deliberation_sink.emit(envelope=envelope, context=ctx)
        except Exception as e:
            logger.debug("Deliberation sink emit failed: %s", e)
    
    async def route_batch(
        self,
        commands: List[GenerativeCommand],
        concurrent: bool = True,
    ) -> List[GeneratedOperation]:
        """Process multiple commands.
        
        Args:
            commands: List of commands to process
            concurrent: If True, process concurrently; otherwise sequential
            
        Returns:
            List of generated operations (same order as input)
        """
        if concurrent:
            return await asyncio.gather(*[self.route(cmd) for cmd in commands])
        else:
            return [await self.route(cmd) for cmd in commands]
    
    def get_stats(self) -> Dict[str, Any]:
        """Get router statistics.

        Back-compat: early integration tests expect cache stats directly under
        top-level keys like `memory` and `persistent`.
        """
        cache_stats: Dict[str, Any] = {}
        if hasattr(self.cache, 'get_stats'):
            cache_stats = self.cache.get_stats()  # type: ignore[assignment]

        stats: Dict[str, Any] = {
            'total_commands_processed': self.total_commands_processed,
            'total_errors': self.total_errors,
            'error_rate': (
                self.total_errors / max(1, self.total_commands_processed) * 100
            ),
            'cache_stats': cache_stats,
            'handlers': self.registry.get_stats(),
        }

        if isinstance(cache_stats, dict) and 'memory' in cache_stats:
            stats['memory'] = cache_stats.get('memory')
            stats['persistent'] = cache_stats.get('persistent')
        else:
            stats['memory'] = {
                'hits': cache_stats.get('hits', 0),
                'misses': cache_stats.get('misses', 0),
            }
            stats['persistent'] = {
                'enabled': False,
                'hits': 0,
                'misses': 0,
            }

        return stats
    
    def list_handlers(self) -> Dict[str, List[str]]:
        """List all registered handlers."""
        return self.registry.list_handlers()
