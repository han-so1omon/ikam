"""IKAM Fragment Algebra V3 – Minimal Fragment Model

V3 keeps three first-class core types:
  1. Fragment  (universal content container)
  2. Artifact  (named entry-point into a fragment DAG; lives in ikam.graph)
  3. ProvenanceEvent  (append-only proof log; lives in ikam.graph)

All higher-level semantics are encoded through MIME-typed fragment payloads
and graph structure – there is no enum-driven type switch.

References:
  - docs/ikam/IKAM_FRAGMENT_ALGEBRA_V3.md  (normative type spec)
  - docs/ikam/IKAM_MONOID_ALGEBRA_CONTRACT.md  (algebraic invariants)

Invariants enforced here:
  F1. At least one of ``fragment_id``, ``cas_id``, or ``value`` SHALL be present.
  F2. (Caller-responsibility) If both are present, ``H(C(value)) == cas_id``.
  F3. ``cas_id`` identity excludes transport/runtime metadata.
"""
from __future__ import annotations

from typing import Any, Dict, List, Optional

from pydantic import BaseModel, Field, model_validator

# ---------------------------------------------------------------------------
# Relation sub-models (payload of fragments with RELATION_MIME)
# ---------------------------------------------------------------------------

RELATION_MIME = "application/ikam-relation+json"
CONCEPT_MIME = "application/ikam-concept+json"


class SlotBinding(BaseModel):
    """Bind a named slot to a specific fragment identity."""
    slot: str
    fragment_id: str


class BindingGroup(BaseModel):
    """One invocation-set for a relation: groups slot bindings under an invocation id."""
    invocation_id: str
    slots: List[SlotBinding] = Field(default_factory=list)


class Relation(BaseModel):
    """Relation payload stored inside a Fragment whose MIME is RELATION_MIME.

    A relation may be pure-semantic (no function) or executable (with function_cas_id).
    """
    predicate: str
    directed: bool = True
    confidence_score: float = Field(default=0.80, ge=0.0, le=1.0)
    qualifiers: Dict[str, Any] = Field(default_factory=dict)

    # None => pure semantic relation (no executable function)
    function_cas_id: Optional[str] = None
    output_mime_type: Optional[str] = None

    # Multiple invocation slot-sets for one relation definition
    binding_groups: List[BindingGroup] = Field(default_factory=list)


# ---------------------------------------------------------------------------
# V3 Fragment (minimal core type)
# ---------------------------------------------------------------------------

class Fragment(BaseModel):
    """Universal content container – the single core type in V3.

    Valid states:
      - Graph-id-only: fragment_id set, cas_id and value absent.
      - CAS-only: cas_id set, value absent.
      - Inline-only: value set, cas_id absent.
      - Dual: both set (inline cache + identity binding).

    Invariant F1: at least one of ``fragment_id``, ``cas_id``, or ``value`` must be present.
    """

    fragment_id: Optional[str] = Field(default=None, description="Logical UUID inode for graph topology")
    cas_id: Optional[str] = Field(default=None, description="Cryptographic hash for byte-level storage deduplication")
    value: Optional[Any] = None
    mime_type: Optional[str] = None

    @model_validator(mode="after")
    def _check_f1_at_least_one(self) -> "Fragment":
        if self.cas_id is None and self.fragment_id is None and self.value is None:
            raise ValueError(
                "Fragment invariant F1 violated: at least one of fragment_id, cas_id, or value must be present"
            )
        return self


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def is_relation_fragment(fragment: Fragment) -> bool:
    """Return True if *fragment* carries a relation payload (by MIME type)."""
    return fragment.mime_type == RELATION_MIME


# This module exports V3 Fragment Algebra types.


__all__ = [
    "Fragment",
    "SlotBinding",
    "BindingGroup",
    "Relation",
    "RELATION_MIME",
    "is_relation_fragment",
]
