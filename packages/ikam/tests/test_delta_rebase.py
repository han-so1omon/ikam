"""Tests for delta chain management and rebase operations.

Validates:
- Delta application is deterministic and lossless
- Rebase preserves reconstruction equality
- Chain length limits are enforced
- CAS deduplication works with rebased artifacts
"""

import pytest
import uuid
from datetime import datetime, timezone

from ikam.delta_chain import (
    DeltaOperation,
    DeltaChain,
    DeltaChainLimitExceeded,
    apply_delta,
    compute_delta,
    rebase_delta_chain,
    check_chain_limit,
    build_delta_chain,
)
from ikam.graph import Artifact, StoredFragment
from ikam.delta_chain import DeltaDerivationRef


class TestDeltaOperations:
    """Test individual delta operations."""
    
    def test_delta_replace_simple(self):
        """Test simple replace operation."""
        base = b"Hello, world!"
        ops = [
            DeltaOperation(
                operation_type="replace",
                position=7,
                old_content=b"world",
                new_content=b"IKAM"
            )
        ]
        
        result = apply_delta(base, ops)
        assert result == b"Hello, IKAM!"
    
    def test_delta_replace_verification_fails(self):
        """Test that delta verification catches mismatches."""
        base = b"Hello, world!"
        ops = [
            DeltaOperation(
                operation_type="replace",
                position=7,
                old_content=b"WRONG",  # Doesn't match actual content
                new_content=b"IKAM"
            )
        ]
        
        with pytest.raises(ValueError, match="Delta verification failed"):
            apply_delta(base, ops)
    
    def test_delta_insert(self):
        """Test insert operation."""
        base = b"Hello!"
        ops = [
            DeltaOperation(
                operation_type="insert",
                position=5,
                new_content=b", world"
            )
        ]
        
        result = apply_delta(base, ops)
        assert result == b"Hello, world!"
    
    def test_delta_delete(self):
        """Test delete operation."""
        base = b"Hello, world!"
        ops = [
            DeltaOperation(
                operation_type="delete",
                position=5,
                old_content=b", world"
            )
        ]
        
        result = apply_delta(base, ops)
        assert result == b"Hello!"
    
    def test_delta_sequence(self):
        """Test sequence of operations with current-relative positions.
        
        Positions refer to the current transformed content after each operation,
        not the original base content.
        """
        base = b"The LTV/CAC is 7.1 with 23% inventory savings."
        ops = [
            DeltaOperation(
                operation_type="replace",
                position=0,
                old_content=b"The LTV/CAC is 7.1",
                new_content=b"LTV/CAC 7.1"
            ),
            # After op1: "LTV/CAC 7.1 with 23% inventory savings." (39 bytes)
            # Position 11 is right after "LTV/CAC 7.1"
            DeltaOperation(
                operation_type="replace",
                position=11,
                old_content=b" with 23% inventory savings",
                new_content="; inventory −23%".encode("utf-8")
            )
        ]
        
        result = apply_delta(base, ops)
        assert result == "LTV/CAC 7.1; inventory −23%.".encode("utf-8")
    
    def test_compute_delta_identity(self):
        """Test delta computation for identical content."""
        content = b"Same content"
        deltas = compute_delta(content, content)
        assert deltas == []
    
    def test_compute_delta_difference(self):
        """Test delta computation for different content."""
        base = b"Hello, world!"
        derived = b"Hello, IKAM!"
        deltas = compute_delta(base, derived)
        
        # Should produce at least one operation
        assert len(deltas) > 0
        
        # Applying delta should reproduce derived content
        result = apply_delta(base, deltas)
        assert result == derived


class TestDeltaChain:
    """Test delta chain management."""
    
    def test_chain_append_and_length(self):
        """Test chain construction and length tracking."""
        chain = DeltaChain(base_artifact_id="base-123")
        
        assert chain.chain_length == 0
        assert not chain.exceeds_limit(3)
        
        chain.append("v1", [])
        assert chain.chain_length == 1
        
        chain.append("v2", [])
        assert chain.chain_length == 2
        
        chain.append("v3", [])
        assert chain.chain_length == 3
        assert not chain.exceeds_limit(3)

        with pytest.raises(DeltaChainLimitExceeded):
            chain.append("v4", [])
    
    def test_check_chain_limit(self):
        """Test chain limit detection from derivation graph."""
        # Build derivation graph: base -> v1 -> v2 -> v3 -> v4
        base_id = str(uuid.uuid4())
        v1_id = str(uuid.uuid4())
        v2_id = str(uuid.uuid4())
        v3_id = str(uuid.uuid4())
        v4_id = str(uuid.uuid4())
        
        derivations = {
            v1_id: DeltaDerivationRef(
                derivation_type="delta",
                source_artifact_ids=[base_id],
                parameters={"operations": []}
            ),
            v2_id: DeltaDerivationRef(
                derivation_type="delta",
                source_artifact_ids=[v1_id],
                parameters={"operations": []}
            ),
            v3_id: DeltaDerivationRef(
                derivation_type="delta",
                source_artifact_ids=[v2_id],
                parameters={"operations": []}
            ),
            v4_id: DeltaDerivationRef(
                derivation_type="delta",
                source_artifact_ids=[v3_id],
                parameters={"operations": []}
            ),
        }
        
        # Check v3 (chain length 3, within limit)
        exceeds, length = check_chain_limit(v3_id, derivations)
        assert length == 3
        assert not exceeds
        
        # Check v4 (chain length 4, exceeds limit)
        exceeds, length = check_chain_limit(v4_id, derivations)
        assert length == 4
        assert exceeds


class TestRebase:
    """Test rebase operation and reconstruction equality."""
    
    def test_rebase_preserves_content(self):
        """Test that rebase preserves final content (lossless guarantee)."""
        # Create base artifact
        base_content = b"Version 0: Original content"
        base_fragment = StoredFragment.from_bytes(base_content)
        base_artifact = Artifact(
            id=str(uuid.uuid4()),
            kind="document",
            title="Base Document",
            root_fragment_id=base_fragment.id
        )
        
        # Build delta chain manually
        v1_content = b"Version 1: Updated content"
        v2_content = b"Version 2: Final content"
        
        delta1 = compute_delta(base_content, v1_content)
        delta2 = compute_delta(v1_content, v2_content)
        
        chain = DeltaChain(base_artifact_id=base_artifact.id)
        chain.append(str(uuid.uuid4()), delta1)
        chain.append(str(uuid.uuid4()), delta2)
        
        # Rebase
        canonical_artifact, canonical_fragments, derivation = rebase_delta_chain(
            base_artifact,
            [base_fragment],
            chain,
            {base_fragment.id: base_fragment}
        )
        
        # Verify reconstruction equality
        rebased_content = b"".join(f.bytes for f in canonical_fragments)
        assert rebased_content == v2_content
        
        # Verify derivation metadata
        assert derivation.derivation_type == "transform"
        assert derivation.parameters["operation"] == "rebase"
        assert derivation.parameters["chain_length"] == 2
        assert derivation.parameters["original_base_id"] == base_artifact.id

    def test_rebase_is_deterministic_for_same_inputs(self):
        """Rebasing the same inputs should produce stable IDs and metadata."""
        base_content = b"Base content"
        base_fragment = StoredFragment.from_bytes(base_content)
        base_artifact = Artifact(
            id=str(uuid.uuid4()),
            kind="document",
            title="Base",
            root_fragment_id=base_fragment.id,
        )

        v1 = b"Derived v1"
        v2 = b"Derived v2"
        chain = DeltaChain(base_artifact_id=base_artifact.id)
        chain.append(str(uuid.uuid4()), compute_delta(base_content, v1))
        chain.append(str(uuid.uuid4()), compute_delta(v1, v2))

        a1, frags1, d1 = rebase_delta_chain(
            base_artifact,
            [base_fragment],
            chain,
            {base_fragment.id: base_fragment},
        )
        a2, frags2, d2 = rebase_delta_chain(
            base_artifact,
            [base_fragment],
            chain,
            {base_fragment.id: base_fragment},
        )

        assert b"".join(f.bytes for f in frags1) == b"".join(f.bytes for f in frags2) == v2
        assert frags1[0].id == frags2[0].id
        assert a1.id == a2.id
        assert d1.id == d2.id
        assert d1.parameters == d2.parameters
    
    def test_rebase_with_empty_chain(self):
        """Test rebase with no deltas (identity operation)."""
        base_content = b"Unchanged content"
        base_fragment = StoredFragment.from_bytes(base_content)
        base_artifact = Artifact(
            id=str(uuid.uuid4()),
            kind="document",
            title="Base",
            root_fragment_id=base_fragment.id
        )
        
        chain = DeltaChain(base_artifact_id=base_artifact.id)
        # No deltas appended
        
        canonical_artifact, canonical_fragments, derivation = rebase_delta_chain(
            base_artifact,
            [base_fragment],
            chain,
            {base_fragment.id: base_fragment}
        )
        
        # Content should be unchanged
        rebased_content = b"".join(f.bytes for f in canonical_fragments)
        assert rebased_content == base_content
        assert derivation.parameters["chain_length"] == 0
    
    def test_rebase_multiple_fragments(self):
        """Test rebase with multi-fragment artifact."""
        # Base artifact composed of 3 fragments
        f1 = StoredFragment.from_bytes(b"Part 1")
        f2 = StoredFragment.from_bytes(b" | Part 2")
        f3 = StoredFragment.from_bytes(b" | Part 3")
        
        base_artifact = Artifact(
            id=str(uuid.uuid4()),
            kind="document",
            title="Multi-Fragment Doc",
            root_fragment_id=f1.id
        )
        
        base_content = b"Part 1 | Part 2 | Part 3"
        derived_content = b"Part 1 | MODIFIED | Part 3"
        
        delta = compute_delta(base_content, derived_content)
        chain = DeltaChain(base_artifact_id=base_artifact.id)
        chain.append(str(uuid.uuid4()), delta)
        
        canonical_artifact, canonical_fragments, derivation = rebase_delta_chain(
            base_artifact,
            [f1, f2, f3],
            chain,
            {f1.id: f1, f2.id: f2, f3.id: f3}
        )
        
        # Verify final content
        rebased_content = b"".join(f.bytes for f in canonical_fragments)
        assert rebased_content == derived_content
    
    def test_rebase_cas_deduplication(self):
        """Test that rebased fragments use CAS (same content -> same ID)."""
        base_content = b"Content A"
        base_fragment = StoredFragment.from_bytes(base_content)
        base_artifact = Artifact(
            id=str(uuid.uuid4()),
            kind="document",
            root_fragment_id=base_fragment.id
        )
        
        # Delta that changes content then changes it back
        intermediate = b"Content B"
        delta1 = compute_delta(base_content, intermediate)
        delta2 = compute_delta(intermediate, base_content)  # Back to original
        
        chain = DeltaChain(base_artifact_id=base_artifact.id)
        chain.append(str(uuid.uuid4()), delta1)
        chain.append(str(uuid.uuid4()), delta2)
        
        canonical_artifact, canonical_fragments, derivation = rebase_delta_chain(
            base_artifact,
            [base_fragment],
            chain,
            {base_fragment.id: base_fragment}
        )
        
        # Final content is back to original
        rebased_content = b"".join(f.bytes for f in canonical_fragments)
        assert rebased_content == base_content
        
        # CAS should produce same fragment ID as original
        assert canonical_fragments[0].id == base_fragment.id


class TestBuildDeltaChain:
    """Test delta chain reconstruction from derivation graph."""
    
    def test_build_chain_from_graph(self):
        """Test building DeltaChain from derivation graph."""
        base_id = str(uuid.uuid4())
        v1_id = str(uuid.uuid4())
        v2_id = str(uuid.uuid4())
        
        derivations = {
            v1_id: DeltaDerivationRef(
                derivation_type="delta",
                source_artifact_ids=[base_id],
                parameters={"operations": [{"type": "edit1"}]}
            ),
            v2_id: DeltaDerivationRef(
                derivation_type="delta",
                source_artifact_ids=[v1_id],
                parameters={"operations": [{"type": "edit2"}]}
            ),
        }
        
        chain = build_delta_chain(v2_id, derivations)
        
        assert chain is not None
        assert chain.base_artifact_id == base_id
        assert chain.chain_length == 2
        assert len(chain.deltas) == 2
    
    def test_build_chain_no_deltas(self):
        """Test building chain from non-delta artifact."""
        artifact_id = str(uuid.uuid4())
        derivations = {
            artifact_id: DeltaDerivationRef(
                derivation_type="compose",  # Not a delta
                source_artifact_ids=[],
                parameters={}
            )
        }
        
        chain = build_delta_chain(artifact_id, derivations)
        assert chain is None


class TestReconstructionEquality:
    """Test the core guarantee: reconstruct(rebase(chain)) = reconstruct(apply_deltas(chain))."""
    
    def test_reconstruction_equality(self):
        """Verify reconstruction equality after rebase."""
        # Base content
        base_content = b"The quick brown fox jumps over the lazy dog."
        base_fragment = StoredFragment.from_bytes(base_content)
        base_artifact = Artifact(
            id=str(uuid.uuid4()),
            kind="document",
            title="Pangram",
            root_fragment_id=base_fragment.id
        )
        
        # Apply 3 deltas sequentially
        v1_content = b"The quick brown fox jumps over the ACTIVE dog."
        v2_content = b"The FAST brown fox jumps over the ACTIVE dog."
        v3_content = b"The FAST brown fox LEAPS over the ACTIVE dog."
        
        delta1 = compute_delta(base_content, v1_content)
        delta2 = compute_delta(v1_content, v2_content)
        delta3 = compute_delta(v2_content, v3_content)
        
        # Manual application
        manual_result = apply_delta(base_content, delta1)
        manual_result = apply_delta(manual_result, delta2)
        manual_result = apply_delta(manual_result, delta3)
        
        # Rebase application
        chain = DeltaChain(base_artifact_id=base_artifact.id)
        chain.append(str(uuid.uuid4()), delta1)
        chain.append(str(uuid.uuid4()), delta2)
        chain.append(str(uuid.uuid4()), delta3)
        
        canonical_artifact, canonical_fragments, derivation = rebase_delta_chain(
            base_artifact,
            [base_fragment],
            chain,
            {base_fragment.id: base_fragment}
        )
        
        rebase_result = b"".join(f.bytes for f in canonical_fragments)
        
        # CRITICAL INVARIANT: byte-level equality
        assert rebase_result == manual_result == v3_content
        
        # Verify derivation records full chain
        assert len(derivation.source_artifact_ids) == 4  # base + 3 deltas
