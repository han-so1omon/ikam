"""Concatenate composition — sequence fragment bytes in order."""
from __future__ import annotations

import base64
from typing import Dict, List

from ikam.fragments import Fragment


def concatenate_compose(
    fragment_ids: List[str],
    fragment_store: Dict[str, Fragment],
) -> bytes:
    """Decode bytes_b64 from each fragment in order, concatenate, return bytes.

    Deterministic: same fragments in same order always produce identical bytes.

    Raises:
        KeyError: if a fragment_id is not found in the store.
    """
    parts: list[bytes] = []
    for fid in fragment_ids:
        frag = fragment_store[fid]  # KeyError if missing
        val = frag.value if isinstance(frag.value, dict) else {}
        b64 = val.get("bytes_b64", "")
        parts.append(base64.b64decode(b64))
    return b"".join(parts)
