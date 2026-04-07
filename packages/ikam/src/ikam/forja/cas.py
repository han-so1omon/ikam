"""Content-Addressable Storage fragment construction.

Public API for creating CAS-addressed Fragments from values.
Delegates canonical serialization and CAS ID derivation to `ikam.adapters`.
"""
from __future__ import annotations

from typing import Any

import ikam.adapters as adapters
from ikam.fragments import Fragment


def cas_fragment(value: Any, mime_type: str) -> Fragment:
    """Create a content-addressed Fragment from a value and MIME type.

    Thin convenience wrapper over the canonical adapter boundary.
    """
    fragment = Fragment(value=value, mime_type=mime_type)
    storage = adapters.v3_to_storage(fragment)
    return Fragment(cas_id=storage.id, value=value, mime_type=mime_type)
