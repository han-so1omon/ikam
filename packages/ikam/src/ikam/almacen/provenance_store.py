from __future__ import annotations

from dataclasses import dataclass
from typing import Protocol, Sequence

from ..provenance import DerivationRecord, DerivationType


@dataclass(frozen=True)
class GroundedArtifact:
    artifact_id: str


class ProvenanceGraphStore(Protocol):
    """Port for provenance traversal.

    Implementations must be pure interfaces here (no DB/graph execution).
    """

    def list_grounded_artifacts_for_concept(
        self,
        *,
        project_id: str,
        concept_id: str,
        derivation_type: str,
        max_edges: int = 500,
    ) -> Sequence[GroundedArtifact]:
        """Return artifacts grounded to a concept within a project."""

    def record_derivation(self, derivation: DerivationRecord) -> str:
        """Record a derivation edge.

        Implementations may persist to an event log, a relational table, or a
        graph projection, but IKAM calls this only through the port.

        Returns an opaque derivation id.
        """

    def list_derivations(
        self,
        *,
        source_key: str | None = None,
        target_key: str | None = None,
        derivation_type: DerivationType | None = None,
    ) -> Sequence[DerivationRecord]:
        """List derivations by source, target, and/or type."""
