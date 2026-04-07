from __future__ import annotations

import re
from typing import Any


def resolve_target_value(payload: dict[str, Any], target: str) -> Any:
    current: Any = payload
    for part in target.split("."):
        if isinstance(current, dict) and part in current:
            current = current[part]
            continue
        return None
    return current


def validate_inline_schema(payload: dict[str, Any], schema_definition: dict[str, Any]) -> tuple[bool, dict[str, Any]]:
    schema_type = str(schema_definition.get("type") or "").strip()
    if schema_type != "object":
        return False, {"reason": "unsupported_schema_type", "schema_type": schema_type}
    required = [str(item) for item in schema_definition.get("required", []) if isinstance(item, str) and item]
    missing = [key for key in required if key not in payload]
    return (len(missing) == 0, {"schema": schema_definition, "missing_keys": missing})


def validate_regex(value: Any, pattern: str, flags: str | None = None) -> tuple[bool, dict[str, Any]]:
    if not isinstance(value, str):
        return False, {"reason": "non_string_value"}
    re_flags = 0
    if isinstance(flags, str) and flags.strip().upper() == "IGNORECASE":
        re_flags |= re.IGNORECASE
    matched = re.search(pattern, value, flags=re_flags) is not None
    return matched, {"pattern": pattern, "flags": flags or ""}
