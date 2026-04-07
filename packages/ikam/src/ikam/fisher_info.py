"""Fisher Information metrics for IKAM v2 provenance tracking.

This module implements Prometheus metrics and validation helpers for the
Fisher Information dominance guarantee:

    I_IKAM(θ) ≥ I_RAG(θ) + Δ_provenance_FI(θ)

Where:
    - I_IKAM(θ): Total Fisher Information with provenance
    - I_RAG(θ): Baseline Fisher Information (flat content only)
    - Δ_provenance_FI(θ): Non-negative provenance increment

Mathematical Framework:
    Fisher Information Chain Rule:
        I((A,Y); θ) = I(A; θ) + E[I(Y; θ | A)]
        
    Where:
        A = artifact content (text, data, visuals)
        Y = provenance (derivation relationships, hierarchy)
        θ = generative parameters (style, intent, structure)
        
    Decomposition by Provenance Type:
        Δ_provenance_FI(θ) = Σ Δ_i where Δ_i ≥ 0 for each edge type:
        
        - Δ_decomposition: Hierarchical constraints (L0 → L1 → L2)
        - Δ_reuse: Cross-artifact consistency (M reuses → (M-1)·I_consistency)
        - Δ_delta: Mutation semantics (if delta correlates with θ)
        - Δ_variation: Variation policy (if Z depends on θ)
        - Δ_structural: Parent-child semantic constraints

References:
    - docs/ikam/FISHER_INFORMATION_GAINS.md (mathematical proofs)
    - docs/ikam/MUTATION_AND_VARIATION_MODEL.md (delta/variation semantics)
    - packages/ikam/src/ikam/provenance.py (data model)

Usage:
    from ikam.fisher_info import (
        FisherInfoMetrics,
        calculate_reuse_contribution,
        validate_fisher_dominance,
    )
    
    # Initialize metrics
    metrics = FisherInfoMetrics()
    
    # Record provenance edge
    metrics.record_derivation(
        derivation_type="reuse",
        fisher_contribution=2.5,  # (M-1) * I_consistency
        metadata={"reuse_count": 3}
    )
    
    # Validate I_IKAM ≥ I_RAG + Δ_provenance
    is_valid = validate_fisher_dominance(
        i_rag=10.0,         # Baseline (flat content)
        i_ikam=12.5,        # IKAM (content + provenance)
        tolerance=1e-6      # Numerical tolerance
    )
    assert is_valid  # Guarantee must hold
"""
from __future__ import annotations

import logging
from dataclasses import dataclass, field
from typing import Dict, List, Optional

from prometheus_client import Counter, Gauge, Histogram

from .provenance import DerivationType

logger = logging.getLogger("ikam.fisher_info")

# Prometheus metrics for Fisher Information tracking
fisher_info_total = Gauge(
    "ikam_fisher_info_total_bits",
    "Total Fisher Information I_IKAM(θ) in bits",
)

fisher_info_rag_baseline = Gauge(
    "ikam_fisher_info_rag_baseline_bits",
    "RAG baseline Fisher Information I_RAG(θ) in bits",
)

fisher_info_provenance_delta = Gauge(
    "ikam_fisher_info_provenance_delta_bits",
    "Provenance increment Δ_provenance_FI(θ) in bits",
)

fisher_info_contributions = Counter(
    "ikam_fisher_info_contributions_total",
    "Count of provenance edges contributing to Fisher Information",
    labelnames=["derivation_type"],
)

fisher_info_contribution_histogram = Histogram(
    "ikam_fisher_info_contribution_bits",
    "Distribution of Fisher Information contributions per edge",
    labelnames=["derivation_type"],
    buckets=(0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0),
)

fisher_dominance_violations = Counter(
    "ikam_fisher_dominance_violations_total",
    "Count of violations of I_IKAM ≥ I_RAG + Δ_provenance guarantee",
)


@dataclass
class FisherInfoBreakdown:
    """Breakdown of Fisher Information by provenance type.
    
    Attributes:
        i_rag: Baseline Fisher Information (flat content) in bits
        decomposition: FI from hierarchical decomposition edges
        reuse: FI from fragment reuse across artifacts
        delta: FI from mutation tracking (deltas)
        variation: FI from render variations
        structural: FI from parent-child relationships
        
    Mathematical Guarantee:
        i_ikam = i_rag + (decomposition + reuse + delta + variation + structural)
        i_ikam ≥ i_rag (monotonicity)
        
    Example:
        breakdown = FisherInfoBreakdown(
            i_rag=10.0,          # Flat content baseline
            reuse=2.5,           # 3 reuses → (3-1) * 1.25 bits
            structural=1.0,      # L0→L1→L2 hierarchy
            decomposition=0.5,   # Fragment boundaries
        )
        assert breakdown.i_ikam == 14.0  # 10.0 + 2.5 + 1.0 + 0.5
        assert breakdown.delta_provenance == 4.0  # Sum of contributions
    """
    
    i_rag: float = 0.0  # Baseline (flat content)
    decomposition: float = 0.0
    reuse: float = 0.0
    delta: float = 0.0
    variation: float = 0.0
    structural: float = 0.0
    
    @property
    def delta_provenance(self) -> float:
        """Total provenance increment Δ_provenance_FI(θ)."""
        return (
            self.decomposition
            + self.reuse
            + self.delta
            + self.variation
            + self.structural
        )
    
    @property
    def i_ikam(self) -> float:
        """Total IKAM Fisher Information I_IKAM(θ)."""
        return self.i_rag + self.delta_provenance
    
    def validate(self, tolerance: float = 1e-6) -> bool:
        """Validate Fisher Information dominance guarantee.
        
        Args:
            tolerance: Numerical tolerance for floating-point comparisons
            
        Returns:
            True if I_IKAM ≥ I_RAG + Δ_provenance within tolerance
            
        Raises:
            ValueError if guarantee is violated (indicates implementation bug)
        """
        if self.delta_provenance < -tolerance:
            raise ValueError(
                f"Provenance increment must be non-negative: {self.delta_provenance:.6f} < 0"
            )
        
        expected_ikam = self.i_rag + self.delta_provenance
        if abs(self.i_ikam - expected_ikam) > tolerance:
            raise ValueError(
                f"Fisher Information chain rule violated: "
                f"I_IKAM={self.i_ikam:.6f} != I_RAG + Δ={expected_ikam:.6f}"
            )
        
        if self.i_ikam < self.i_rag - tolerance:
            raise ValueError(
                f"Fisher Information dominance violated: "
                f"I_IKAM={self.i_ikam:.6f} < I_RAG={self.i_rag:.6f}"
            )
        
        return True
    
    def to_dict(self) -> Dict[str, float]:
        """Convert to dictionary for logging/reporting."""
        return {
            "i_rag": self.i_rag,
            "i_ikam": self.i_ikam,
            "delta_provenance": self.delta_provenance,
            "decomposition": self.decomposition,
            "reuse": self.reuse,
            "delta": self.delta,
            "variation": self.variation,
            "structural": self.structural,
        }


class FisherInfoMetrics:
    """Prometheus metrics collector for Fisher Information tracking.
    
    This class maintains a running breakdown of Fisher Information
    contributions and exposes Prometheus metrics for monitoring.
    
    Thread Safety:
        Prometheus client handles thread-safe metric updates.
        Internal breakdown uses simple assignment (not thread-safe for reads).
    """
    
    def __init__(self):
        self.breakdown = FisherInfoBreakdown()
    
    def set_rag_baseline(self, i_rag: float) -> None:
        """Set baseline Fisher Information from flat content.
        
        Args:
            i_rag: RAG baseline in bits (must be non-negative)
        """
        if i_rag < 0:
            raise ValueError(f"Fisher Information cannot be negative: {i_rag}")
        
        self.breakdown.i_rag = i_rag
        fisher_info_rag_baseline.set(i_rag)
        self._update_totals()
    
    def record_derivation(
        self,
        derivation_type: DerivationType,
        fisher_contribution: float,
        metadata: Optional[Dict] = None,
    ) -> None:
        """Record a provenance derivation edge and its FI contribution.
        
        Args:
            derivation_type: Type of derivation (reuse, delta, etc.)
            fisher_contribution: Δ_i for this edge in bits (must be ≥ 0)
            metadata: Optional metadata (logged but not stored)
            
        Raises:
            ValueError if fisher_contribution < 0 (violates non-negativity)
        """
        if fisher_contribution < 0:
            raise ValueError(
                f"Fisher Information contribution cannot be negative: {fisher_contribution}"
            )
        
        # Update breakdown by type
        if derivation_type == DerivationType.DECOMPOSITION:
            self.breakdown.decomposition += fisher_contribution
        elif derivation_type == DerivationType.REUSE:
            self.breakdown.reuse += fisher_contribution
        elif derivation_type == DerivationType.DELTA:
            self.breakdown.delta += fisher_contribution
        elif derivation_type == DerivationType.VARIATION:
            self.breakdown.variation += fisher_contribution
        elif derivation_type == DerivationType.STRUCTURAL:
            self.breakdown.structural += fisher_contribution
        
        # Update Prometheus metrics
        fisher_info_contributions.labels(derivation_type=derivation_type.value).inc()
        fisher_info_contribution_histogram.labels(
            derivation_type=derivation_type.value
        ).observe(fisher_contribution)
        
        self._update_totals()
        
        logger.debug(
            f"Recorded {derivation_type.value} edge: +{fisher_contribution:.3f} bits "
            f"(total Δ={self.breakdown.delta_provenance:.3f})"
        )
    
    def _update_totals(self) -> None:
        """Update Prometheus gauges for total FI and provenance delta."""
        fisher_info_total.set(self.breakdown.i_ikam)
        fisher_info_provenance_delta.set(self.breakdown.delta_provenance)
    
    def validate(self, tolerance: float = 1e-6) -> bool:
        """Validate Fisher Information dominance guarantee.
        
        Returns:
            True if all guarantees hold
            
        Raises:
            ValueError if guarantee violated (increments violation counter)
        """
        try:
            return self.breakdown.validate(tolerance=tolerance)
        except ValueError as e:
            fisher_dominance_violations.inc()
            logger.error(f"Fisher Information guarantee violated: {e}")
            raise
    
    def get_breakdown(self) -> FisherInfoBreakdown:
        """Get current Fisher Information breakdown."""
        return self.breakdown


# Helper functions for Fisher Information calculations

def calculate_reuse_contribution(
    reuse_count: int,
    i_consistency: float = 1.25,
) -> float:
    """Calculate Fisher Information contribution from fragment reuse.
    
    Mathematical Model:
        When a fragment is reused M times across artifacts, the reuse
        edges add consistency constraints that improve parameter estimation.
        
        I_reuse(θ) ≈ (M - 1) · I_consistency(θ)
        
    Where:
        M = reuse_count (number of artifacts using this fragment)
        I_consistency = information gained from each consistency constraint
        
    Args:
        reuse_count: Number of artifacts using this fragment (M ≥ 1)
        i_consistency: FI per consistency edge (default 1.25 bits)
        
    Returns:
        Fisher Information contribution in bits (≥ 0)
        
    Example:
        # Fragment reused in 3 artifacts
        fi = calculate_reuse_contribution(reuse_count=3)
        # Returns: (3 - 1) * 1.25 = 2.5 bits
    """
    if reuse_count < 1:
        raise ValueError(f"Reuse count must be ≥ 1: {reuse_count}")
    
    return max(0.0, (reuse_count - 1) * i_consistency)


def calculate_hierarchy_contribution(
    hierarchy_depth: int,
    i_per_level: float = 0.5,
) -> float:
    """Calculate Fisher Information contribution from hierarchical structure.
    
    Mathematical Model:
        Hierarchical decomposition (L0 → L1 → L2 → ...) adds semantic
        constraints at each level transition.
        
        I_hierarchy(θ) ≈ depth · I_per_level(θ)
        
    Args:
        hierarchy_depth: Number of hierarchy levels (e.g., 3 for L0→L1→L2)
        i_per_level: FI per level transition (default 0.5 bits)
        
    Returns:
        Fisher Information contribution in bits (≥ 0)
        
    Example:
        # 3-level hierarchy (L0 → L1 → L2)
        fi = calculate_hierarchy_contribution(hierarchy_depth=3)
        # Returns: 3 * 0.5 = 1.5 bits
    """
    if hierarchy_depth < 0:
        raise ValueError(f"Hierarchy depth cannot be negative: {hierarchy_depth}")
    
    return hierarchy_depth * i_per_level


def validate_fisher_dominance(
    i_rag: float,
    i_ikam: float,
    tolerance: float = 1e-6,
) -> bool:
    """Validate Fisher Information dominance guarantee.
    
    Mathematical Guarantee:
        I_IKAM(θ) ≥ I_RAG(θ) + Δ_provenance_FI(θ)
        where Δ_provenance_FI(θ) ≥ 0
        
    Equivalent to:
        I_IKAM(θ) ≥ I_RAG(θ)
        
    Args:
        i_rag: RAG baseline Fisher Information (bits)
        i_ikam: IKAM Fisher Information with provenance (bits)
        tolerance: Numerical tolerance for floating-point comparisons
        
    Returns:
        True if guarantee holds within tolerance
        
    Raises:
        ValueError if I_IKAM < I_RAG (guarantee violated)
    """
    if i_ikam < i_rag - tolerance:
        fisher_dominance_violations.inc()
        raise ValueError(
            f"Fisher Information dominance violated: "
            f"I_IKAM={i_ikam:.6f} < I_RAG={i_rag:.6f}"
        )
    
    return True
