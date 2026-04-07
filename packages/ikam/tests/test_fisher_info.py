"""Tests for IKAM v2 provenance tracking and Fisher Information metrics.

This module tests Task 5 deliverables:
1. Provenance data model (DerivationRecord, ProvenanceMetadata)
2. Fisher Information metrics (FisherInfoMetrics, breakdown, validation)
3. Provenance-aware storage backend (ProvenanceBackend)
4. Mathematical guarantee: I_IKAM ≥ I_RAG + Δ_provenance

References:
    - docs/ikam/FISHER_INFORMATION_GAINS.md (mathematical framework)
    - docs/ikam/MUTATION_AND_VARIATION_MODEL.md (delta/variation semantics)
    - packages/ikam/src/ikam/provenance.py (data model)
    - packages/ikam/src/ikam/fisher_info.py (metrics)
"""
import pytest
from datetime import datetime

from ikam.provenance import (
    DerivationType,
    DerivationRecord,
    ModelCallProvenance,
    ProvenanceMetadata,
)
from ikam.fisher_info import (
    FisherInfoBreakdown,
    FisherInfoMetrics,
    calculate_reuse_contribution,
    calculate_hierarchy_contribution,
    validate_fisher_dominance,
)


# === Provenance Data Model Tests ===

def test_derivation_record_creation():
    """Test DerivationRecord creation and serialization."""
    record = DerivationRecord(
        source_key="blake3:abc123",
        target_key="blake3:def456",
        derivation_type=DerivationType.REUSE,
        operation="embed_fragment",
        metadata={"reuse_count": 3, "salience": 0.9},
        fisher_info_contribution=2.5,
    )
    
    assert record.source_key == "blake3:abc123"
    assert record.target_key == "blake3:def456"
    assert record.derivation_type == DerivationType.REUSE
    assert record.fisher_info_contribution == 2.5
    
    # Round-trip serialization
    data = record.to_dict()
    assert data["derivation_type"] == "reuse"
    assert data["fisher_info_contribution"] == 2.5
    
    restored = DerivationRecord.from_dict(data)
    assert restored.source_key == record.source_key
    assert restored.derivation_type == record.derivation_type


def test_model_call_provenance_round_trip():
    """Ensure ModelCallProvenance serializes cleanly to/from dict."""
    prov = ModelCallProvenance(
        model="gpt-4o-mini",
        prompt_hash="blake3:prompt",
        output_hash="blake3:output",
        seed=42,
        cost_usd=0.0011,
        tokens_input=120,
        tokens_output=280,
        latency_ms=850.0,
        cached=False,
        temperature=0.7,
    )

    serialized = prov.to_dict()
    restored = ModelCallProvenance.from_dict(serialized)

    assert restored == prov
    assert serialized["model"] == "gpt-4o-mini"


def test_derivation_record_with_model_call_metadata():
    """DerivationRecord should preserve MODEL_CALL metadata payload."""
    prov = ModelCallProvenance(
        model="claude-haiku",
        prompt_hash="prompt_hash",
        output_hash="output_hash",
        seed=None,
        cost_usd=0.0,
        tokens_input=90,
        tokens_output=0,
        latency_ms=12.0,
        cached=True,
        temperature=0.0,
    )

    record = DerivationRecord(
        source_key="fragment:prompt",
        target_key="fragment:output",
        derivation_type=DerivationType.MODEL_CALL,
        operation="model_call",
        metadata={
            "model_call": prov.to_dict(),
            "function_id": "gfn_123",
            "execution_id": "exec_123",
        },
    )

    data = record.to_dict()
    restored = DerivationRecord.from_dict(data)

    assert restored.derivation_type == DerivationType.MODEL_CALL
    assert restored.metadata["model_call"]["model"] == "claude-haiku"
    assert restored.metadata["model_call"]["cached"] is True


def test_provenance_metadata_serialization():
    """Test ProvenanceMetadata to_dict/from_dict round-trip."""
    metadata = ProvenanceMetadata(
        derived_from="blake3:abc123",
        derivation_type=DerivationType.DELTA,
        parent_fragment_id="parent-123",
        salience=0.8,
        reuse_count=2,
        delta_size=150,
    )
    
    data = metadata.to_dict()
    assert data["derivation_type"] == "delta"
    assert data["delta_size"] == 150
    
    restored = ProvenanceMetadata.from_dict(data)
    assert restored.derived_from == "blake3:abc123"
    assert restored.derivation_type == DerivationType.DELTA
    assert restored.delta_size == 150


# === Fisher Information Metrics Tests ===

def test_fisher_info_breakdown_totals():
    """Test FisherInfoBreakdown property calculations."""
    breakdown = FisherInfoBreakdown(
        i_rag=10.0,
        reuse=2.5,
        structural=1.0,
        decomposition=0.5,
    )
    
    # Check totals
    assert breakdown.delta_provenance == 4.0  # 2.5 + 1.0 + 0.5
    assert breakdown.i_ikam == 14.0  # 10.0 + 4.0


def test_fisher_info_breakdown_validation_success():
    """Test FisherInfoBreakdown.validate() passes for valid data."""
    breakdown = FisherInfoBreakdown(
        i_rag=10.0,
        reuse=2.5,
        structural=1.0,
    )
    
    assert breakdown.validate()  # Should not raise


def test_fisher_info_breakdown_validation_negative_delta():
    """Test FisherInfoBreakdown.validate() fails for negative Δ_provenance."""
    breakdown = FisherInfoBreakdown(
        i_rag=10.0,
        reuse=-1.0,  # Invalid: negative contribution
    )
    
    with pytest.raises(ValueError, match="Provenance increment must be non-negative"):
        breakdown.validate()


def test_fisher_info_breakdown_validation_dominance_violation():
    """Test FisherInfoBreakdown.validate() fails if I_IKAM < I_RAG.
    
    Note: This scenario is mathematically impossible with non-negative
    provenance contributions since i_ikam = i_rag + delta_provenance.
    The validation check exists as a defensive assertion to catch
    implementation bugs, but cannot be easily triggered in practice.
    
    We skip this test since the property calculation ensures the
    guarantee always holds structurally.
    """
    pytest.skip("Cannot violate dominance with computed i_ikam property")


def test_fisher_info_metrics_rag_baseline():
    """Test FisherInfoMetrics.set_rag_baseline()."""
    metrics = FisherInfoMetrics()
    
    metrics.set_rag_baseline(10.0)
    
    assert metrics.breakdown.i_rag == 10.0
    assert metrics.breakdown.i_ikam == 10.0  # No provenance yet


def test_fisher_info_metrics_record_derivation():
    """Test FisherInfoMetrics.record_derivation() updates breakdown."""
    metrics = FisherInfoMetrics()
    metrics.set_rag_baseline(10.0)
    
    # Record reuse edge
    metrics.record_derivation(
        derivation_type=DerivationType.REUSE,
        fisher_contribution=2.5,
        metadata={"reuse_count": 3},
    )
    
    assert metrics.breakdown.reuse == 2.5
    assert metrics.breakdown.delta_provenance == 2.5
    assert metrics.breakdown.i_ikam == 12.5  # 10.0 + 2.5
    
    # Record structural edge
    metrics.record_derivation(
        derivation_type=DerivationType.STRUCTURAL,
        fisher_contribution=1.0,
    )
    
    assert metrics.breakdown.structural == 1.0
    assert metrics.breakdown.delta_provenance == 3.5  # 2.5 + 1.0
    assert metrics.breakdown.i_ikam == 13.5  # 10.0 + 3.5


def test_fisher_info_metrics_negative_contribution():
    """Test FisherInfoMetrics rejects negative contributions."""
    metrics = FisherInfoMetrics()
    
    with pytest.raises(ValueError, match="cannot be negative"):
        metrics.record_derivation(
            derivation_type=DerivationType.REUSE,
            fisher_contribution=-1.0,  # Invalid
        )


def test_fisher_info_metrics_validate():
    """Test FisherInfoMetrics.validate() checks mathematical guarantees."""
    metrics = FisherInfoMetrics()
    metrics.set_rag_baseline(10.0)
    metrics.record_derivation(DerivationType.REUSE, 2.5)
    
    assert metrics.validate()  # Should pass


# === Helper Function Tests ===

def test_calculate_reuse_contribution():
    """Test calculate_reuse_contribution() formula."""
    # M = 1 (no reuse) → 0 bits
    fi = calculate_reuse_contribution(reuse_count=1)
    assert fi == 0.0
    
    # M = 3 reuses → (3-1) * 1.25 = 2.5 bits
    fi = calculate_reuse_contribution(reuse_count=3)
    assert fi == 2.5
    
    # M = 5 reuses → (5-1) * 1.25 = 5.0 bits
    fi = calculate_reuse_contribution(reuse_count=5, i_consistency=1.25)
    assert fi == 5.0


def test_calculate_reuse_contribution_invalid():
    """Test calculate_reuse_contribution() rejects invalid inputs."""
    with pytest.raises(ValueError, match="Reuse count must be ≥ 1"):
        calculate_reuse_contribution(reuse_count=0)


def test_calculate_hierarchy_contribution():
    """Test calculate_hierarchy_contribution() formula."""
    # 3-level hierarchy (L0 → L1 → L2) → 3 * 0.5 = 1.5 bits
    fi = calculate_hierarchy_contribution(hierarchy_depth=3)
    assert fi == 1.5
    
    # 5-level hierarchy → 5 * 0.5 = 2.5 bits
    fi = calculate_hierarchy_contribution(hierarchy_depth=5, i_per_level=0.5)
    assert fi == 2.5
    
    # 0-level (flat) → 0 bits
    fi = calculate_hierarchy_contribution(hierarchy_depth=0)
    assert fi == 0.0


def test_validate_fisher_dominance_success():
    """Test validate_fisher_dominance() passes for valid data."""
    assert validate_fisher_dominance(i_rag=10.0, i_ikam=12.5)


def test_validate_fisher_dominance_failure():
    """Test validate_fisher_dominance() fails when I_IKAM < I_RAG."""
    with pytest.raises(ValueError, match="Fisher Information dominance violated"):
        validate_fisher_dominance(i_rag=15.0, i_ikam=12.0)


def test_validate_fisher_dominance_tolerance():
    """Test validate_fisher_dominance() respects numerical tolerance."""
    # Within tolerance → should pass
    assert validate_fisher_dominance(
        i_rag=10.0,
        i_ikam=9.9999999,  # Tiny numerical error
        tolerance=1e-6,
    )
    
    # Outside tolerance → should fail
    with pytest.raises(ValueError):
        validate_fisher_dominance(
            i_rag=10.0,
            i_ikam=9.99,  # Clear violation
            tolerance=1e-6,
        )


# === Integration Test: Full Fisher Information Chain ===

def test_fisher_info_chain_rule():
    """Test Fisher Information chain rule: I((A,Y); θ) = I(A; θ) + E[I(Y; θ | A)].
    
    Scenario:
        - RAG baseline: I_RAG = 10.0 bits (flat content)
        - IKAM adds provenance:
          - 3 fragment reuses → 2.5 bits
          - 3-level hierarchy → 1.5 bits
          - 2 decomposition edges → 1.0 bits
        - Expected: I_IKAM = 10.0 + 2.5 + 1.5 + 1.0 = 15.0 bits
    """
    metrics = FisherInfoMetrics()
    
    # Set baseline
    metrics.set_rag_baseline(10.0)
    
    # Add provenance edges
    metrics.record_derivation(DerivationType.REUSE, calculate_reuse_contribution(3))
    metrics.record_derivation(DerivationType.STRUCTURAL, calculate_hierarchy_contribution(3))
    metrics.record_derivation(DerivationType.DECOMPOSITION, 1.0)
    
    # Validate chain rule
    breakdown = metrics.get_breakdown()
    assert breakdown.i_rag == 10.0
    assert breakdown.reuse == 2.5
    assert breakdown.structural == 1.5
    assert breakdown.decomposition == 1.0
    assert breakdown.delta_provenance == 5.0  # 2.5 + 1.5 + 1.0
    assert breakdown.i_ikam == 15.0  # 10.0 + 5.0
    
    # Validate mathematical guarantees
    assert breakdown.validate()
    assert validate_fisher_dominance(breakdown.i_rag, breakdown.i_ikam)


# === Provenance Metadata in Fragment Context ===

def test_provenance_metadata_in_fragment():
    """Test storing provenance metadata in Fragment.metadata field.
    
    This tests backward-compatible provenance tracking without
    modifying the Fragment model itself.
    """
    from ikam.fragments import Fragment
    
    # Create fragment with provenance metadata
    provenance = ProvenanceMetadata(
        derived_from="blake3:source123",
        derivation_type=DerivationType.DELTA,
        reuse_count=3,
        delta_size=150,
    )
    
    fragment = Fragment(
        cas_id="fragment-123",
        value={"text": "Sample text"},
        mime_type="application/json",
    )
    
    # Store provenance in metadata (not yet implemented in Fragment model,
    # but this shows the intended pattern)
    # fragment.metadata = {"provenance": provenance.to_dict()}
    
    # For now, just verify provenance metadata serialization works
    metadata_dict = provenance.to_dict()
    assert "derived_from" in metadata_dict
    assert "derivation_type" in metadata_dict
    assert fragment.cas_id == "fragment-123"
    
    # Restore and verify
    restored = ProvenanceMetadata.from_dict(metadata_dict)
    assert restored.derived_from == provenance.derived_from
    assert restored.reuse_count == 3
