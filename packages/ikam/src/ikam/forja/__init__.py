"""Forja package exports for map-first ingestion and reconstruction."""
from __future__ import annotations


class DecompositionError(RuntimeError):
    """Raised when structural map generation cannot proceed."""


from .reconstructor import (
    reconstruct_document,
    reconstruct_binary,
    ReconstructionError,
)
from .enricher import (
    EntityRelationEnricher,
    ENTITY_MIME,
)
from .contracts import (
    ExtractionBatch,
    ExtractedEntity,
    ExtractedRelation,
)
from .cas import cas_fragment


__all__ = [
    "reconstruct_document",
    "reconstruct_binary",
    "DecompositionError",
    "ReconstructionError",
    "EntityRelationEnricher",
    "ENTITY_MIME",
    "ExtractionBatch",
    "ExtractedEntity",
    "ExtractedRelation",
    "cas_fragment",
]
