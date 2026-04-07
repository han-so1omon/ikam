"""Overlay composition — apply delta atop base fragment."""
from __future__ import annotations

from ikam.fragments import Fragment
from ikam.forja.cas import cas_fragment


def overlay_compose(base: Fragment, delta: Fragment) -> Fragment:
    """Apply delta values over base values. Deterministic."""
    base_val = dict(base.value) if isinstance(base.value, dict) else {}
    delta_val = dict(delta.value) if isinstance(delta.value, dict) else {}

    merged = {**base_val, **delta_val}
    mime = base.mime_type or delta.mime_type or "application/octet-stream"
    return cas_fragment(merged, mime)
