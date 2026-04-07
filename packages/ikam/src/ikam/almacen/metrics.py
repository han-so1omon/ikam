"""Observability helpers for IKAM storage metrics.

This module provides functions to record storage metrics for IKAM fragments
and artifacts, enabling monitoring of storage efficiency (Δ(N) gains) and
deduplication effectiveness.

Mathematical Guarantees:
- Δ(N) = bytes_flat - bytes_fragmented ≥ 0 for N ≥ 2 artifacts
- bytes_flat: cumulative bytes if artifacts stored without fragmentation
- bytes_fragmented: cumulative bytes in CAS storage (deduplicated)
- Monotonicity: Δ(N+1) ≥ Δ(N) when artifacts share fragments

Usage:
    from ikam.almacen.metrics import record_storage_observation
    
    # After storing an artifact
    record_storage_observation(
        project_id="proj_123",
        artifact_type="slide",
        flat_bytes=1024,      # size if stored as flat blob
        fragmented_bytes=512  # actual size in CAS (deduplicated)
    )
"""

from __future__ import annotations

import importlib
import logging
import os
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    pass

logger = logging.getLogger(__name__)


def record_storage_observation(
    project_id: str,
    artifact_type: str,
    flat_bytes: int,
    fragmented_bytes: int,
) -> None:
    """Record storage metrics for an artifact.
    
    This function updates Prometheus metrics tracking storage efficiency:
    - Increments flat_bytes counter (baseline without deduplication)
    - Increments fragmented_bytes counter (actual CAS storage)
    - Updates delta_bytes gauge (storage savings)
    
    Parameters:
        project_id: Project identifier (for labeling)
        artifact_type: Type of artifact (e.g., "slide", "sheet", "text")
        flat_bytes: Size in bytes if artifact stored without fragmentation
        fragmented_bytes: Actual size in CAS storage after deduplication
    
    Mathematical Properties:
        - Δ = flat_bytes - fragmented_bytes ≥ 0 (storage savings)
        - Cumulative Δ(N) is monotonically increasing for N ≥ 2 artifacts
        - Zero savings when N=1 (no deduplication opportunities)
    
    Example:
        >>> # Store first artifact (no dedup yet)
        >>> record_storage_observation("p1", "slide", 1000, 1000)  # Δ=0
        >>> 
        >>> # Store second artifact sharing 50% fragments
        >>> record_storage_observation("p1", "slide", 1000, 500)   # Δ=500
        >>> 
        >>> # Cumulative Δ(2) = 500 ≥ Δ(1) = 0 ✓ monotonic
    """
    try:
        hook_path = os.getenv("IKAM_STORAGE_METRICS_HOOK", "").strip()
        if not hook_path:
            raise ImportError("IKAM_STORAGE_METRICS_HOOK is not configured")
        module_name, func_name = hook_path.rsplit(":", 1)
        record_ikam_storage_observation = getattr(importlib.import_module(module_name), func_name)
        
        record_ikam_storage_observation(
            project_id=project_id,
            artifact_type=artifact_type,
            flat_bytes=flat_bytes,
            ikam_bytes=fragmented_bytes,
        )
    except (ImportError, ValueError, AttributeError):
        logger.debug(
            "IKAM storage metrics not recorded (no external metrics hook configured): "
            f"project={project_id} type={artifact_type} flat={flat_bytes} frag={fragmented_bytes}"
        )
    except Exception as exc:
        # Non-fatal: metrics recording should never break core functionality
        logger.warning(
            f"Failed to record IKAM storage metrics: {exc}",
            exc_info=True,
        )


def validate_delta_monotonicity(
    deltas: list[tuple[int, int]],
) -> tuple[bool, list[str]]:
    """Validate that Δ(N) is monotonically increasing with artifact count N.
    
    Parameters:
        deltas: List of (N, delta_bytes) tuples sorted by N
    
    Returns:
        (is_monotonic, violations): Tuple of:
            - is_monotonic: True if Δ(N+1) ≥ Δ(N) for all pairs
            - violations: List of human-readable violation messages
    
    Mathematical Property:
        For IKAM v2 fragmentation to provide guaranteed storage gains,
        we require: Δ(N+1) ≥ Δ(N) ∀N ≥ 1
    
    Example:
        >>> deltas = [(1, 0), (2, 512), (3, 1024)]
        >>> is_mono, viols = validate_delta_monotonicity(deltas)
        >>> assert is_mono is True
        >>> assert len(viols) == 0
        
        >>> deltas_bad = [(1, 0), (2, 512), (3, 256)]  # Δ(3) < Δ(2)
        >>> is_mono, viols = validate_delta_monotonicity(deltas_bad)
        >>> assert is_mono is False
        >>> assert "N=3: Δ=256 < Δ(2)=512" in viols[0]
    """
    if not deltas:
        return True, []
    
    violations: list[str] = []
    prev_n, prev_delta = deltas[0]
    
    for n, delta in deltas[1:]:
        if delta < prev_delta:
            violations.append(
                f"N={n}: Δ={delta} < Δ({prev_n})={prev_delta} "
                f"(violation of monotonicity guarantee)"
            )
        prev_n, prev_delta = n, delta
    
    return len(violations) == 0, violations


__all__ = [
    "record_storage_observation",
    "validate_delta_monotonicity",
]
