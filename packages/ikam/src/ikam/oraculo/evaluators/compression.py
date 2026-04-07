"""Compression evaluator — fragment and byte-level deduplication metrics."""
from __future__ import annotations

from ikam.oraculo.graph_state import GraphState
from ikam.oraculo.reports import CompressionReport


class CompressionEvaluator:
    """Measures CAS compression: fragment counts, byte savings, dedup ratio."""

    def evaluate(self, graph_state: GraphState) -> CompressionReport:
        total = graph_state.fragment_count()
        unique = graph_state.unique_fragment_count()
        total_bytes = graph_state.total_bytes()
        unique_bytes = graph_state.unique_bytes()

        dedup_ratio = 1.0 - (unique / total) if total > 0 else 0.0
        byte_savings = 1.0 - (unique_bytes / total_bytes) if total_bytes > 0 else 0.0

        return CompressionReport(
            total_fragments=total,
            unique_fragments=unique,
            dedup_ratio=dedup_ratio,
            total_bytes=total_bytes,
            unique_bytes=unique_bytes,
            byte_savings=byte_savings,
        )
