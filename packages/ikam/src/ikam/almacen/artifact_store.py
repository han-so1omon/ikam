from __future__ import annotations

from typing import Protocol, Sequence


class ArtifactStore(Protocol):
    def upsert_artifact_with_fragments(
        self,
        *,
        artifact_id: str,
        kind: str,
        title: str | None,
        created_at,
        fragment_ids: Sequence[str],
    ) -> None:
        """Idempotently upsert artifact row and replace fragment membership.

        Implementations must execute within the caller-owned transaction.
        """

    def get_fragment_ids_for_artifact(self, *, artifact_id: str) -> list[str]:
        """Return CAS fragment ids in stable position order."""
