"""Provenance projection: latest-evaluation lookup from append-only event log.

Per IKAM_FRAGMENT_ALGEBRA_V3.md §5 (Provenance Projection Model):

- Source of truth: append-only ProvenanceEvent log
- Fast lookup: latest_output(relation_fragment_id, invocation_id) -> Optional[output_cas_id]
- Ordering key: provenance timestamp (monotonic)
- Rebuildability: projection can be reconstructed from provenance log only

This module provides an in-memory projection that can be:
1. Built incrementally via apply() as events arrive
2. Rebuilt from scratch via rebuild_projection() over a full event stream
"""

from __future__ import annotations

import datetime as dt
from dataclasses import dataclass, field
from typing import Any, Optional


@dataclass
class ProjectionEntry:
    """A single projected latest-output record."""

    output_cas_id: str
    timestamp: dt.datetime
    environment: dict[str, Any] = field(default_factory=dict)


class LatestOutputProjection:
    """In-memory projection of latest evaluation outputs.

    Keyed by (relation_fragment_id, invocation_id), stores the most recent
    output_cas_id and associated environment metadata.
    """

    def __init__(self) -> None:
        self._entries: dict[tuple[str, str], ProjectionEntry] = {}

    def apply(self, event: dict[str, Any]) -> None:
        """Apply a single provenance event to the projection.

        Only events with action='evaluated' and valid metadata are projected.
        Later timestamps overwrite earlier ones for the same key.
        """
        if event.get("action") != "evaluated":
            return

        metadata = event.get("metadata") or {}
        invocation_id = metadata.get("invocation_id")
        output_cas_id = metadata.get("output_cas_id")
        fragment_id = event.get("fragment_id")

        if not (fragment_id and invocation_id and output_cas_id):
            return

        key = (fragment_id, invocation_id)
        timestamp = event.get("timestamp", dt.datetime.min.replace(tzinfo=dt.timezone.utc))
        existing = self._entries.get(key)

        if existing is None or timestamp > existing.timestamp:
            self._entries[key] = ProjectionEntry(
                output_cas_id=output_cas_id,
                timestamp=timestamp,
                environment=metadata.get("environment") or {},
            )

    def latest_output(self, relation_fragment_id: str, invocation_id: str) -> Optional[str]:
        """Return the latest output_cas_id for a (relation, invocation) pair, or None."""
        entry = self._entries.get((relation_fragment_id, invocation_id))
        return entry.output_cas_id if entry else None

    def get_entry(self, relation_fragment_id: str, invocation_id: str) -> Optional[ProjectionEntry]:
        """Return the full projection entry including environment metadata."""
        return self._entries.get((relation_fragment_id, invocation_id))


def rebuild_projection(events: list[dict[str, Any]]) -> LatestOutputProjection:
    """Rebuild projection from a complete ordered event stream.

    Events should be ordered by timestamp ascending for correct last-write-wins.
    """
    proj = LatestOutputProjection()
    for event in events:
        proj.apply(event)
    return proj
