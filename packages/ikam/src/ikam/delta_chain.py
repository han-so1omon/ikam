"""Delta chain management and rebase operations for IKAM graph.

This module implements the L ≤ 3 delta chain policy with deterministic rebase
to maintain bounded reconstruction complexity while preserving lossless guarantees.
"""

from __future__ import annotations

import datetime as dt
import uuid
from dataclasses import dataclass, field as dc_field
from typing import Any, List, Optional, Tuple

try:
    from pydantic import BaseModel, Field
except Exception:  # pragma: no cover
    from pydantic.v1 import BaseModel, Field  # type: ignore

from .graph import Artifact, StoredFragment


DEFAULT_MAX_DELTA_CHAIN_LENGTH = 3


@dataclass(frozen=True)
class RebaseDerivation:
    """Result metadata from a delta chain rebase operation.

    Replaces the old Derivation model for rebase-specific use.
    """

    id: str
    derivation_type: str  # "transform" for rebase
    source_fragment_ids: list[str] = dc_field(default_factory=list)
    source_artifact_ids: list[str] = dc_field(default_factory=list)
    target_artifact_id: str = ""
    parameters: dict[str, Any] = dc_field(default_factory=dict)
    created_at: dt.datetime = dc_field(default_factory=lambda: dt.datetime.now(dt.timezone.utc))


@dataclass(frozen=True)
class DeltaDerivationRef:
    """Minimal reference to a derivation for chain traversal.

    Used by check_chain_limit and build_delta_chain to traverse
    derivation graphs without depending on the full Derivation model.
    """

    derivation_type: str
    source_artifact_ids: list[str] = dc_field(default_factory=list)
    parameters: dict[str, Any] = dc_field(default_factory=dict)


class DeltaChainLimitExceeded(ValueError):
    def __init__(self, *, max_length: int, chain_length: int) -> None:
        super().__init__(
            f"Delta chain length {chain_length} exceeds maximum allowed length {max_length}"
        )
        self.max_length = max_length
        self.chain_length = chain_length


class DeltaOperation(BaseModel):
    """A deterministic delta transformation.
    
    Represents the minimal change needed to transform a base content to a derived content.
    Must be reproducible and lossless.
    """
    
    operation_type: str  # "replace", "insert", "delete", "patch"
    position: int = 0  # Byte offset or index
    old_content: bytes = Field(default=b"")  # Original content (for verification)
    new_content: bytes = Field(default=b"")  # Replacement content
    metadata: dict = Field(default_factory=dict)  # Algorithm version, encoding, etc.


class DeltaChain(BaseModel):
    """A sequence of deltas forming a derivation chain.
    
    Invariant: reconstruct(apply_deltas(base, chain)) preserves byte-level equality
    """
    
    base_artifact_id: str  # Canonical base artifact
    deltas: List[Tuple[str, List[DeltaOperation]]] = Field(default_factory=list)  # [(artifact_id, operations)]
    chain_length: int = 0
    
    def append(
        self,
        artifact_id: str,
        operations: List[DeltaOperation],
        *,
        max_length: int = DEFAULT_MAX_DELTA_CHAIN_LENGTH,
        allow_exceed_limit: bool = False,
    ) -> None:
        """Add a delta to the chain, enforcing the bounded-length policy."""
        next_length = self.chain_length + 1
        if (not allow_exceed_limit) and next_length > max_length:
            raise DeltaChainLimitExceeded(max_length=max_length, chain_length=next_length)
        self.deltas.append((artifact_id, operations))
        self.chain_length = next_length
    
    def exceeds_limit(self, max_length: int = DEFAULT_MAX_DELTA_CHAIN_LENGTH) -> bool:
        """Check if chain exceeds maximum allowed length."""
        return self.chain_length > max_length


def apply_delta(base_content: bytes, operations: List[DeltaOperation]) -> bytes:
    """Apply a sequence of delta operations to base content.
    
    Deterministic and lossless transformation.
    """
    current = base_content
    
    for op in operations:
        if op.operation_type == "replace":
            actual = current[op.position:op.position + len(op.old_content)]
            if actual != op.old_content:
                raise ValueError(
                    f"Delta verification failed at position {op.position}: "
                    f"expected {op.old_content[:20]!r}, got {actual[:20]!r}"
                )
            current = (
                current[:op.position]
                + op.new_content
                + current[op.position + len(op.old_content):]
            )
        elif op.operation_type == "insert":
            current = current[:op.position] + op.new_content + current[op.position:]
        elif op.operation_type == "delete":
            current = current[:op.position] + current[op.position + len(op.old_content):]
        elif op.operation_type == "patch":
            raise NotImplementedError("Patch operation requires custom implementation")
        else:
            raise ValueError(f"Unknown operation type: {op.operation_type}")
    
    return current


def compute_delta(base_content: bytes, derived_content: bytes) -> List[DeltaOperation]:
    """Compute minimal delta operations transforming base to derived."""
    if base_content == derived_content:
        return []
    
    return [
        DeltaOperation(
            operation_type="replace",
            position=0,
            old_content=base_content,
            new_content=derived_content,
            metadata={"algorithm": "naive_replace", "version": "1.0.0"}
        )
    ]


def rebase_delta_chain(
    base_artifact: Artifact,
    base_fragments: List[StoredFragment],
    chain: DeltaChain,
    all_fragments: dict[str, StoredFragment]  # fragment_id -> StoredFragment lookup
) -> Tuple[Artifact, List[StoredFragment], RebaseDerivation]:
    """Collapse delta chain into canonical base artifact.
    
    Ensures: reconstruct(rebase(chain)) = reconstruct(apply_deltas(chain))
    """
    base_content = b"".join(f.bytes for f in base_fragments)
    
    current_content = base_content
    source_artifact_ids = [base_artifact.id]
    
    for artifact_id, operations in chain.deltas:
        current_content = apply_delta(current_content, operations)
        source_artifact_ids.append(artifact_id)
    
    canonical_fragment = StoredFragment.from_bytes(current_content)
    
    canonical_artifact_id = uuid.uuid5(
        uuid.NAMESPACE_URL,
        f"ikam/rebase:artifact:{base_artifact.id}:{canonical_fragment.id}",
    )
    canonical_artifact = Artifact(
        id=str(canonical_artifact_id),
        kind=base_artifact.kind,
        title=f"{base_artifact.title or 'Artifact'} (rebased)",
        root_fragment_id=canonical_fragment.id,
        created_at=dt.datetime.now(dt.timezone.utc)
    )
    
    rebase_derivation_id = uuid.uuid5(
        uuid.NAMESPACE_URL,
        f"ikam/rebase:derivation:{base_artifact.id}:{canonical_fragment.id}:chain:{chain.chain_length}",
    )
    rebase_derivation = RebaseDerivation(
        id=str(rebase_derivation_id),
        derivation_type="transform",
        source_fragment_ids=[f.id for f in base_fragments],
        source_artifact_ids=source_artifact_ids,
        target_artifact_id=canonical_artifact.id,
        parameters={
            "operation": "rebase",
            "chain_length": chain.chain_length,
            "original_base_id": base_artifact.id,
        },
        created_at=dt.datetime.now(dt.timezone.utc)
    )
    
    return canonical_artifact, [canonical_fragment], rebase_derivation


def check_chain_limit(
    artifact_id: str,
    derivation_graph: dict[str, DeltaDerivationRef]
) -> Tuple[bool, int]:
    """Check if artifact is at the end of a delta chain exceeding L=3."""
    chain_length = 0
    current_id = artifact_id
    
    while current_id in derivation_graph:
        deriv = derivation_graph[current_id]
        if deriv.derivation_type != "delta":
            break
        chain_length += 1
        if deriv.source_artifact_ids:
            current_id = deriv.source_artifact_ids[0]
        else:
            break
    
    return chain_length > DEFAULT_MAX_DELTA_CHAIN_LENGTH, chain_length


def build_delta_chain(
    artifact_id: str,
    derivation_graph: dict[str, DeltaDerivationRef],
    *,
    max_length: int = DEFAULT_MAX_DELTA_CHAIN_LENGTH,
) -> Optional[DeltaChain]:
    """Build DeltaChain object from derivation graph."""
    chain_artifacts = []
    current_id = artifact_id
    
    while current_id in derivation_graph:
        deriv = derivation_graph[current_id]
        if deriv.derivation_type != "delta":
            break
        chain_artifacts.append(current_id)
        if deriv.source_artifact_ids:
            current_id = deriv.source_artifact_ids[0]
        else:
            break
    
    if not chain_artifacts:
        return None
    
    chain = DeltaChain(base_artifact_id=current_id)
    
    for aid in reversed(chain_artifacts):
        deriv = derivation_graph[aid]
        operations = deriv.parameters.get("operations", [])
        chain.append(aid, operations, max_length=max_length, allow_exceed_limit=True)
    
    return chain
