from __future__ import annotations

import datetime as dt
from typing import Literal, Optional

try:
    from pydantic import BaseModel, Field
except Exception:  # pragma: no cover
    # Fallback for environments where pydantic v1 is present
    from pydantic.v1 import BaseModel, Field  # type: ignore

from blake3 import blake3


def _cas_hex(data: bytes) -> str:
    """Compute content-addressable ID as hex digest using blake3.

    Returns a lowercase hex string suitable for use as a stable Fragment ID.
    """
    return blake3(data).hexdigest()


class StoredFragment(BaseModel):
    """A content-addressed, immutable fragment of an artifact.

    **STORAGE LAYER ONLY**: This model represents the minimal CAS storage layer.
    For domain logic (decomposition, rendering, provenance), use `ikam.fragments.Fragment`.
    
    The two-layer architecture:
    - `graph.StoredFragment` (this class): storage/persistence (id, bytes, mime_type, size)
    - `fragments.Fragment`: runtime/domain fragment (fragment_id, cas_id, value, mime_type)
    
    Adapters in `ikam.adapters` handle bidirectional transformation.
    See AGENTS.md § IKAM Fragment Models for usage guidelines.

    Fields:
    - id: blake3 hex digest of `bytes` (CAS)
    - bytes: raw content payload (lossless reconstruction requirement)
    - mime_type: media type of the bytes
    - size: byte length (denormalized convenience)
    """

    id: str
    bytes: bytes
    mime_type: str = Field(default="application/octet-stream")
    size: int

    @classmethod
    def from_bytes(cls, payload: bytes, mime_type: str = "application/octet-stream") -> "StoredFragment":
        fid = _cas_hex(payload)
        return cls(id=fid, bytes=payload, mime_type=mime_type, size=len(payload))


ArtifactKind = Literal[
    "document",
    "sheet",
    "slide_deck",
    "image",
    "video",
    "file",
]


class Artifact(BaseModel):
    """A first-class IKAM artifact with a root fragment entrypoint.

    ``root_fragment_id`` is the DAG entrypoint for reconstruction
    (see IKAM_FRAGMENT_ALGEBRA_V3.md §2.3).
    """

    id: str  # UUID-like external id; generation occurs in application layer
    kind: ArtifactKind
    title: Optional[str] = None
    root_fragment_id: Optional[str] = None
    created_at: dt.datetime = Field(default_factory=lambda: dt.datetime.now(dt.timezone.utc))


class ProvenanceEvent(BaseModel):
    """User/action provenance event to ensure completeness and auditability."""

    id: str
    artifact_id: Optional[str] = None
    fragment_id: Optional[str] = None
    actor_id: Optional[str] = None
    action: str
    timestamp: dt.datetime = Field(default_factory=lambda: dt.datetime.now(dt.timezone.utc))
    metadata: dict = Field(default_factory=dict)


__all__ = [
    "_cas_hex",
    "StoredFragment",
    "Artifact",
    "ArtifactKind",
    "ProvenanceEvent",
]
