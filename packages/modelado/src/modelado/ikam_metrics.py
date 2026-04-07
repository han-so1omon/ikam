"""
IKAM v2 Observability Metrics

Prometheus metrics for monitoring IKAM storage architecture performance and guarantees.
Tracks fragment operations, storage efficiency, provenance completeness, and Fisher Information.

Metrics Categories:
1. Volume metrics: artifact_count, fragment_count, derivation_count
2. Operation metrics: provenance_event_count, reconstruction operations
3. Performance metrics: reconstruction_latency_seconds, rendering_latency_seconds
4. Efficiency metrics: cas_hit_rate, op_shape_reuse_count
5. Guarantees: fisher_info_delta (I_IKAM - I_RAG)

Integration:
- Import in repository layer (ikam_graph_repository.py)
- Increment counters on insert/update operations
- Record histograms for reconstruction and rendering
- Export via /metrics endpoint in base-api

References:
- AGENTS.md: Mathematical soundness guarantees
- docs/observability/: Grafana dashboard configuration

Version: 1.0.0 (IKAM v2 MVP - November 2025)
"""

from __future__ import annotations

from contextlib import contextmanager
from typing import Any, Optional
import time

Counter: Any
Gauge: Any
Histogram: Any
Info: Any

try:
    from prometheus_client import Counter as _Counter
    from prometheus_client import Gauge as _Gauge
    from prometheus_client import Histogram as _Histogram
    from prometheus_client import Info as _Info

    Counter = _Counter
    Gauge = _Gauge
    Histogram = _Histogram
    Info = _Info
except ImportError:
    # Fallback for environments without prometheus_client
    # Metrics become no-ops
    class _NoOpValue:
        def get(self):
            return 0

    class _NoOpMetric:
        def __init__(self):
            self._value = _NoOpValue()

        def inc(self, *args, **kwargs):
            pass
        
        def dec(self, *args, **kwargs):
            pass
        
        def set(self, *args, **kwargs):
            pass
        
        def observe(self, *args, **kwargs):
            pass

        def info(self, *args, **kwargs):
            pass
        
        def labels(self, *args, **kwargs):
            return self
    
    def _no_op_metric(*args, **kwargs):
        return _NoOpMetric()

    Counter = _no_op_metric  # type: ignore[assignment]
    Gauge = _no_op_metric  # type: ignore[assignment]
    Histogram = _no_op_metric  # type: ignore[assignment]
    Info = _no_op_metric  # type: ignore[assignment]


# ============================================================================
# Volume Metrics (State of the system)
# ============================================================================

artifact_count = Gauge(
    "ikam_artifact_count",
    "Total number of artifacts in the graph",
    ["artifact_type"],
)

fragment_count = Gauge(
    "ikam_fragment_count",
    "Total number of fragments stored (CAS deduplication applied)",
    ["fragment_type"],
)

fragment_meta_count = Gauge(
    "ikam_fragment_meta_count",
    "Total number of fragment metadata records (may exceed fragment_count due to reuse)",
)

derivation_count = Counter(
    "ikam_derivation_count",
    "Total number of derivations created",
    ["derivation_type"],
)

# ============================================================================
# Provenance Metrics (Event tracking)
# ============================================================================

provenance_event_count = Counter(
    "ikam_provenance_event_count",
    "Total number of provenance events emitted",
    ["action_type"],
)

provenance_event_errors = Counter(
    "ikam_provenance_event_errors",
    "Failed provenance event emissions (constraint violations, etc.)",
    ["error_type"],
)

# ============================================================================
# Performance Metrics (Latency histograms)
# ============================================================================

reconstruction_latency_seconds = Histogram(
    "ikam_reconstruction_latency_seconds",
    "Time to reconstruct artifact from fragments (lossless operation)",
    buckets=[0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0],
)

rendering_latency_seconds = Histogram(
    "ikam_rendering_latency_seconds",
    "Time to render artifact via external renderer (LaTeX, Canvas, etc.)",
    ["renderer_type"],
    buckets=[0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0],
)

repository_operation_latency_seconds = Histogram(
    "ikam_repository_operation_latency_seconds",
    "Database operation latency (insert, query)",
    ["operation"],
    buckets=[0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0],
)

# ============================================================================
# Efficiency Metrics (Storage and deduplication)
# ============================================================================

cas_hit_rate = Gauge(
    "ikam_cas_hit_rate",
    "Fragment CAS hit rate (0.0-1.0): ratio of deduplicated fragments to total inserts",
)

cas_hits_total = Counter(
    "ikam_cas_hits_total",
    "Total number of CAS hits (fragment already exists, deduplication applied)",
)

cas_misses_total = Counter(
    "ikam_cas_misses_total",
    "Total number of CAS misses (new fragment inserted)",
)

op_shape_reuse_count = Counter(
    "ikam_op_shape_reuse_count",
    "Total number of times an operation shape was reused (IR/DSL deduplication)",
    ["op_type"],
)

# ============================================================================
# Mathematical Guarantee Metrics
# ============================================================================

fisher_info_delta = Gauge(
    "ikam_fisher_info_delta",
    "Estimated Fisher Information delta: I_IKAM - I_RAG (positive indicates provenance gain)",
    ["artifact_type"],
)

storage_delta_bytes = Gauge(
    "ikam_storage_delta_bytes",
    "Storage savings: S_flat - S_IKAM (positive indicates space savings from deduplication)",
    ["artifact_type"],
)

reconstruction_error_count = Counter(
    "ikam_reconstruction_error_count",
    "Failed reconstructions (should be 0 for lossless guarantee)",
    ["error_type"],
)

# ============================================================================
# System Info
# ============================================================================

ikam_info = Info(
    "ikam_version",
    "IKAM v2 storage architecture version and configuration",
)

ikam_info.info({
    "version": "2.0.0-mvp",
    "storage_layer": "postgres-pgvector",
    "cas_algorithm": "blake3",
    "adapter_version": "1.0",
})


# ============================================================================
# Utility Functions
# ============================================================================

def update_cas_hit_rate():
    """Calculate and update CAS hit rate from cumulative counters."""
    hits = cas_hits_total._value.get()
    misses = cas_misses_total._value.get()
    total = hits + misses
    
    if total > 0:
        hit_rate = hits / total
        cas_hit_rate.set(hit_rate)
    else:
        cas_hit_rate.set(0.0)


@contextmanager
def track_reconstruction_latency():
    """Context manager to track reconstruction operation latency."""
    start = time.perf_counter()
    try:
        yield
    finally:
        duration = time.perf_counter() - start
        reconstruction_latency_seconds.observe(duration)


@contextmanager
def track_rendering_latency(renderer_type: str):
    """Context manager to track rendering operation latency."""
    start = time.perf_counter()
    try:
        yield
    finally:
        duration = time.perf_counter() - start
        rendering_latency_seconds.labels(renderer_type=renderer_type).observe(duration)


@contextmanager
def track_repository_operation(operation: str):
    """Context manager to track database operation latency."""
    start = time.perf_counter()
    try:
        yield
    finally:
        duration = time.perf_counter() - start
        repository_operation_latency_seconds.labels(operation=operation).observe(duration)


def record_cas_hit():
    """Record a CAS cache hit (fragment already exists)."""
    cas_hits_total.inc()
    update_cas_hit_rate()


def record_cas_miss():
    """Record a CAS cache miss (new fragment inserted)."""
    cas_misses_total.inc()
    update_cas_hit_rate()


def record_provenance_event(action_type: str):
    """Record a provenance event emission."""
    provenance_event_count.labels(action_type=action_type).inc()


def record_provenance_error(error_type: str):
    """Record a failed provenance event emission."""
    provenance_event_errors.labels(error_type=error_type).inc()


def record_derivation(derivation_type: str):
    """Record a derivation creation."""
    derivation_count.labels(derivation_type=derivation_type).inc()


def record_op_shape_reuse(op_type: str):
    """Record an operation shape reuse (IR/DSL deduplication)."""
    op_shape_reuse_count.labels(op_type=op_type).inc()


def record_reconstruction_error(error_type: str):
    """Record a reconstruction failure (should be rare/zero for lossless guarantee)."""
    reconstruction_error_count.labels(error_type=error_type).inc()


def set_fisher_info_delta(artifact_type: str, delta: float):
    """
    Update Fisher Information delta estimate.
    
    Delta = I_IKAM - I_RAG, where:
    - I_IKAM: Fisher Information with full provenance
    - I_RAG: Fisher Information without provenance (baseline RAG)
    
    Positive delta indicates IKAM's advantage in parameter estimation precision.
    """
    fisher_info_delta.labels(artifact_type=artifact_type).set(delta)


def set_storage_delta(artifact_type: str, delta_bytes: int):
    """
    Update storage savings delta.
    
    Delta = S_flat - S_IKAM, where:
    - S_flat: Flat storage (no deduplication)
    - S_IKAM: IKAM storage (with CAS + metadata deduplication)
    
    Positive delta indicates space savings from deduplication.
    """
    storage_delta_bytes.labels(artifact_type=artifact_type).set(delta_bytes)


def update_volume_metrics(connection):
    """
    Query database to update volume gauge metrics.
    
    Call this periodically (e.g., every 60s) or after bulk operations.
    """
    with connection.cursor() as cx:
        # Artifact counts by type
        cx.execute("""
            SELECT artifact_type, COUNT(*) 
            FROM ikam_artifacts 
            GROUP BY artifact_type
        """)
        for artifact_type, count in cx.fetchall():
            artifact_count.labels(artifact_type=artifact_type).set(count)
        
        # Fragment count (CAS deduplicated)
        cx.execute("SELECT COUNT(*) FROM ikam_fragments")
        total_fragments = cx.fetchone()[0]
        fragment_count.labels(fragment_type="all").set(total_fragments)
        
        # Fragment metadata count (may exceed fragments due to reuse)
        cx.execute("SELECT COUNT(*) FROM ikam_fragment_meta")
        total_meta = cx.fetchone()[0]
        fragment_meta_count.set(total_meta)
        
        # Fragment counts by type
        cx.execute("""
            SELECT type, COUNT(*) 
            FROM ikam_fragment_meta 
            GROUP BY type
        """)
        for frag_type, count in cx.fetchall():
            fragment_count.labels(fragment_type=frag_type).set(count)
