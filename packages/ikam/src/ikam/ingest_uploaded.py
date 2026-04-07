from __future__ import annotations

from typing import Tuple

from .graph import Artifact, StoredFragment


def ingest_uploaded_file(payload: bytes, mime_type: str, artifact_id: str, title: str | None = None) -> Tuple[Artifact, StoredFragment]:
    """Create an IKAM `Artifact(kind='file')` and a CAS `StoredFragment` from raw bytes.

    Returns `(artifact, fragment)`. The caller persists them using the storage layer.
    """

    fragment = StoredFragment.from_bytes(payload, mime_type=mime_type)
    artifact = Artifact(id=artifact_id, kind="file", title=title, root_fragment_id=fragment.id)
    return artifact, fragment
