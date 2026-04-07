"""IngestionPolicy — controls what gets committed during artifact ingestion."""
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class IngestionPolicy:
    """Controls what gets committed during artifact ingestion.

    retain_original_bytes: If True, original imported bytes are stored as
        a CAS fragment connected via 'original-bytes-of' predicate.
        Default False — verified reconstruction makes originals redundant.
    """
    retain_original_bytes: bool = False
