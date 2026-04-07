"""Convert Fragments to embeddable text representations."""
from __future__ import annotations

import json

from ikam.fragments import Fragment


def fragment_to_text(fragment: Fragment) -> str:
    """Convert a Fragment into a text string suitable for embedding.

    Prefixes with MIME type for cross-type discrimination.
    """
    mime = fragment.mime_type or "application/octet-stream"
    prefix = f"[{mime}]"

    if fragment.value is None:
        return f"{prefix} ref:{fragment.cas_id or 'unknown'}"

    v = fragment.value

    # If value has a "text" key, use that directly
    if isinstance(v, dict) and "text" in v:
        return f"{prefix} {v['text']}"

    # If value is a dict, compact JSON
    if isinstance(v, dict):
        # Skip bytes_b64 for readability
        filtered = {k: val for k, val in v.items() if k != "bytes_b64"}
        if filtered:
            return f"{prefix} {json.dumps(filtered, ensure_ascii=False, default=str)}"
        return f"{prefix} [binary content]"

    # Fallback: str representation
    return f"{prefix} {str(v)}"
