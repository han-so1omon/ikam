"""Format composition — apply format spec to raw value."""
from __future__ import annotations

from ikam.fragments import Fragment
from ikam.forja.cas import cas_fragment


def format_compose(raw: Fragment, format_spec: Fragment) -> Fragment:
    """Apply format specification to a raw value fragment. Deterministic."""
    raw_val = dict(raw.value) if isinstance(raw.value, dict) else {}
    fmt_val = dict(format_spec.value) if isinstance(format_spec.value, dict) else {}

    fmt_string = fmt_val.get("format", "")
    raw_number = raw_val.get("raw_value")

    formatted = str(raw_number)  # Default: string representation
    if raw_number is not None and "currency" in fmt_string:
        parts = fmt_string.split(":")
        if len(parts) >= 3:
            decimals = int(parts[2].replace("dp", "")) if "dp" in parts[2] else 2
            formatted = f"${raw_number:,.{decimals}f}"

    merged = {**raw_val, "formatted": formatted, "format_spec": fmt_string}
    mime = raw.mime_type or "application/ikam-value-cell+json"
    return cas_fragment(merged, mime)
