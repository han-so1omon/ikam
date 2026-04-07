"""
Batch Queue for Model Calls: Deterministic ordering with cost/token tracking.

This module implements a stable FIFO batch queue for model calls with:
- Deterministic ordering (stable sort by queue_position)
- Batch grouping (same model + params)
- Per-item cost/token tracking
- Backpressure caps (max batch size, max wait window)
- Idempotent CAS writes (same prompt_hash + seed = same batch slot)

Stable batch queue implementation.
"""

from __future__ import annotations

import hashlib
import logging
from dataclasses import dataclass, field
from datetime import datetime, timedelta
from typing import Any, Optional
from uuid import uuid4

from pydantic import BaseModel, Field, ConfigDict, PrivateAttr

from .model_call_client import ModelCallParams, ModelName

logger = logging.getLogger(__name__)


@dataclass
class BatchItem:
    """Single item in a batch queue."""
    item_id: str
    batch_id: str
    queue_position: int  # Stable FIFO position (for deterministic ordering)
    item_index: int  # Index within batch (for tie-breaking)
    params: ModelCallParams
    param_hash: str  # Hash of (model, prompt, hyperparams) — excludes seed
    prompt_hash: str
    seed_hash: Optional[str]  # Hash of seed (if provided)
    cost_tokens: float = 0.0  # Estimated token count
    cost_usd: float = 0.0  # Estimated cost in USD
    cached: bool = False
    created_at: datetime = field(default_factory=datetime.utcnow)
    
    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for provenance recording."""
        return {
            "item_id": self.item_id,
            "batch_id": self.batch_id,
            "queue_position": self.queue_position,
            "item_index": self.item_index,
            "model": self.params.model.value,
            "param_hash": self.param_hash,
            "prompt_hash": self.prompt_hash,
            "seed_hash": self.seed_hash,
            "cost_tokens": self.cost_tokens,
            "cost_usd": self.cost_usd,
            "cached": self.cached,
            "created_at": self.created_at.isoformat(),
        }


@dataclass
class Batch:
    """A batch of model calls (grouped by model + params)."""
    batch_id: str
    model: ModelName
    param_hash: str  # Hash of (model + prompt + hyperparams)
    items: list[BatchItem] = field(default_factory=list)
    created_at: datetime = field(default_factory=datetime.utcnow)
    submitted_at: Optional[datetime] = None
    completed_at: Optional[datetime] = None
    total_cost_usd: float = 0.0
    total_cost_tokens: float = 0.0
    cached_items: int = 0
    
    def add_item(self, item: BatchItem) -> None:
        """Add item to batch (maintains stable FIFO within batch)."""
        self.items.append(item)
        # Update batch totals
        self.total_cost_usd += item.cost_usd
        self.total_cost_tokens += item.cost_tokens
        if item.cached:
            self.cached_items += 1
    
    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for provenance."""
        return {
            "batch_id": self.batch_id,
            "model": self.model.value,
            "param_hash": self.param_hash,
            "item_count": len(self.items),
            "cached_items": self.cached_items,
            "total_cost_usd": self.total_cost_usd,
            "total_cost_tokens": self.total_cost_tokens,
            "created_at": self.created_at.isoformat(),
            "submitted_at": self.submitted_at.isoformat() if self.submitted_at else None,
            "completed_at": self.completed_at.isoformat() if self.completed_at else None,
        }


class ModelCallBatchQueue(BaseModel):
    """
    Deterministic batch queue for model calls with stable FIFO ordering.

    Responsibilities:
    - Group calls by (model, params_hash) into batches
    - Maintain stable FIFO ordering (queue_position) for deterministic replay
    - Track per-item cost/tokens
    - Enforce backpressure caps (max batch size, max wait window)
    - Support idempotent CAS writes (same prompt_hash + seed = reuse existing item)
    """

    model_config = ConfigDict(arbitrary_types_allowed=True)

    queue_id: str = Field(default_factory=lambda: str(uuid4()))
    
    # Batches keyed by param_hash (for grouping)
    _batches: dict[str, Batch] = PrivateAttr(default_factory=dict)
    
    # Global FIFO position counter (for stable ordering across all batches)
    _global_queue_position: int = PrivateAttr(default=0)
    
    # Idempotent write cache: (param_hash, prompt_hash, seed_hash) → item_id
    _item_cache: dict[str, str] = PrivateAttr(default_factory=dict)
    
    # Configuration
    max_batch_size: int = Field(default=32)  # Max items per batch
    max_wait_window_seconds: int = Field(default=60)  # Max time to wait before submitting
    backpressure_threshold: int = Field(default=100)  # Max total items in queue
    
    _submitted_batches: list[str] = PrivateAttr(default_factory=list)

    async def enqueue(
        self,
        params: ModelCallParams,
        cost_tokens: float = 0.0,
        cost_usd: float = 0.0,
    ) -> BatchItem:
        """
        Enqueue a model call (idempotent).

        If the same prompt_hash + seed combo already exists, returns the existing item.

        Args:
            params: Model call parameters
            cost_tokens: Estimated token count
            cost_usd: Estimated cost

        Returns:
            BatchItem with queue metadata

        Raises:
            ValueError: If backpressure threshold exceeded
        """
        # Compute hashes
        param_hash = params.param_hash()
        prompt_hash = hashlib.sha256(params.prompt.encode()).hexdigest()
        seed_hash = hashlib.sha256(str(params.seed).encode()).hexdigest() if params.seed else None

        # Idempotent write: check if already enqueued
        cache_key = f"{param_hash}|{prompt_hash}|{seed_hash}"
        if cache_key in self._item_cache:
            existing_item_id = self._item_cache[cache_key]
            # Find and return the item
            for batch in self._batches.values():
                for item in batch.items:
                    if item.item_id == existing_item_id:
                        logger.debug(f"Idempotent enqueue (reuse): {existing_item_id[:8]}...")
                        return item
            # Item not found (shouldn't happen), continue with new enqueue

        # Check backpressure
        total_items = sum(len(b.items) for b in self._batches.values())
        if total_items >= self.backpressure_threshold:
            raise ValueError(
                f"Batch queue backpressure exceeded: {total_items} items >= {self.backpressure_threshold}"
            )

        # Get or create batch for this param_hash
        if param_hash not in self._batches:
            batch = Batch(
                batch_id=str(uuid4()),
                model=params.model,
                param_hash=param_hash,
            )
            self._batches[param_hash] = batch
        else:
            batch = self._batches[param_hash]

        # Create item with global queue position
        item = BatchItem(
            item_id=str(uuid4()),
            batch_id=batch.batch_id,
            queue_position=self._global_queue_position,
            item_index=len(batch.items),  # Index within batch
            params=params,
            param_hash=param_hash,
            prompt_hash=prompt_hash,
            seed_hash=seed_hash,
            cost_tokens=cost_tokens,
            cost_usd=cost_usd,
        )

        batch.add_item(item)
        self._global_queue_position += 1

        # Cache for idempotent writes
        self._item_cache[cache_key] = item.item_id

        logger.info(
            f"Enqueued model call: batch={batch.batch_id[:8]}... "
            f"queue_pos={item.queue_position} item_index={item.item_index} "
            f"cost=${cost_usd:.6f}"
        )

        return item

    async def submit_batch(self, param_hash: str) -> Batch:
        """
        Submit a batch for processing.

        Args:
            param_hash: Hash identifying the batch

        Returns:
            Submitted Batch

        Raises:
            ValueError: If batch not found
        """
        if param_hash not in self._batches:
            raise ValueError(f"Batch with param_hash {param_hash} not found")

        batch = self._batches[param_hash]
        batch.submitted_at = datetime.utcnow()
        self._submitted_batches.append(batch.batch_id)

        logger.info(
            f"Submitted batch: {batch.batch_id[:8]}... "
            f"items={len(batch.items)} cost=${batch.total_cost_usd:.6f}"
        )

        return batch

    def get_batch(self, param_hash: str) -> Optional[Batch]:
        """Retrieve a batch by param_hash."""
        return self._batches.get(param_hash)

    def get_all_batches(self) -> list[Batch]:
        """Get all batches in stable FIFO order (by creation time)."""
        return sorted(self._batches.values(), key=lambda b: b.created_at)

    def get_pending_batches(self) -> list[Batch]:
        """Get batches not yet submitted."""
        return [b for b in self._batches.values() if b.submitted_at is None]

    def get_stats(self) -> dict[str, Any]:
        """Get queue statistics."""
        total_items = sum(len(b.items) for b in self._batches.values())
        total_cost = sum(b.total_cost_usd for b in self._batches.values())
        pending_batches = self.get_pending_batches()

        return {
            "queue_id": self.queue_id,
            "total_batches": len(self._batches),
            "submitted_batches": len(self._submitted_batches),
            "pending_batches": len(pending_batches),
            "total_items": total_items,
            "total_cost_usd": total_cost,
            "global_queue_position": self._global_queue_position,
            "cache_entries": len(self._item_cache),
        }

    def clear(self) -> None:
        """Clear all batches and caches."""
        self._batches.clear()
        self._item_cache.clear()
        self._submitted_batches.clear()
        self._global_queue_position = 0
