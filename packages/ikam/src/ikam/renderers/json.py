"""
JSON deterministic renderer for IKAM artifacts.

Honors environment variables:
- IKAM_DETERMINISTIC_RENDER: Enable deterministic mode
- IKAM_FROZEN_TIMESTAMP: ISO8601 timestamp to freeze all dates
- IKAM_STABLE_IDS: Sort keys lexicographically
- IKAM_FLOAT_PRECISION: Round floats to N decimal places
"""

import os
import json
from datetime import datetime
from typing import Any, Dict, List, Union
from uuid import uuid4


def _env_bool(key: str, default: bool = False) -> bool:
    """Parse boolean from environment variable."""
    val = os.getenv(key, "").strip().lower()
    if not val:
        return default
    return val in ("1", "true", "yes", "on")


def _env_int(key: str, default: int) -> int:
    """Parse integer from environment variable."""
    val = os.getenv(key, "").strip()
    if not val:
        return default
    try:
        return int(val)
    except ValueError:
        return default


def _parse_timestamp(ts_str: str) -> datetime:
    """Parse ISO8601 timestamp."""
    try:
        return datetime.fromisoformat(ts_str.replace("Z", "+00:00"))
    except ValueError as e:
        raise ValueError(f"Invalid timestamp format: {ts_str}. Expected ISO8601.") from e


def _round_floats(obj: Any, precision: int) -> Any:
    """Recursively round all floats in data structure."""
    if isinstance(obj, float):
        return round(obj, precision)
    elif isinstance(obj, dict):
        return {k: _round_floats(v, precision) for k, v in obj.items()}
    elif isinstance(obj, list):
        return [_round_floats(item, precision) for item in obj]
    else:
        return obj


def _inject_variance_token(data: Dict[str, Any]) -> Dict[str, Any]:
    """Inject UUID token to ensure non-deterministic outputs differ."""
    data["_variance_token"] = str(uuid4())
    return data


def _apply_deterministic_properties(
    data: Dict[str, Any],
    frozen_timestamp: datetime | None,
    float_precision: int,
) -> Dict[str, Any]:
    """Apply deterministic transformations to JSON data."""
    # Freeze timestamps if specified
    if frozen_timestamp:
        # Normalize to Z notation for consistency
        iso_str = frozen_timestamp.isoformat().replace("+00:00", "Z")
        
        def freeze_timestamps(obj: Any) -> Any:
            if isinstance(obj, dict):
                result = {}
                for k, v in obj.items():
                    # Replace common timestamp fields
                    if k in ("created_at", "updated_at", "timestamp", "date", "modified"):
                        result[k] = iso_str
                    else:
                        result[k] = freeze_timestamps(v)
                return result
            elif isinstance(obj, list):
                return [freeze_timestamps(item) for item in obj]
            else:
                return obj
        
        data = freeze_timestamps(data)
    
    # Round floats if precision specified
    if float_precision >= 0:
        data = _round_floats(data, float_precision)
    
    return data


def render_json(
    data: Union[Dict[str, Any], List[Any]],
    indent: int = 2,
) -> bytes:
    """
    Render JSON with optional deterministic mode.
    
    Args:
        data: Dictionary or list to serialize
        indent: Indentation level (default 2 spaces)
    
    Returns:
        JSON bytes
    
    Environment Variables:
        IKAM_DETERMINISTIC_RENDER: Enable deterministic mode
        IKAM_FROZEN_TIMESTAMP: ISO8601 timestamp to freeze dates
        IKAM_STABLE_IDS: Sort keys lexicographically
        IKAM_FLOAT_PRECISION: Round floats to N decimals (default 6)
    """
    deterministic = _env_bool("IKAM_DETERMINISTIC_RENDER", False)
    frozen_ts_str = os.getenv("IKAM_FROZEN_TIMESTAMP", "").strip()
    stable_ids = _env_bool("IKAM_STABLE_IDS", False)
    float_precision = _env_int("IKAM_FLOAT_PRECISION", 6)
    
    # Parse frozen timestamp if provided
    frozen_ts = None
    if frozen_ts_str:
        frozen_ts = _parse_timestamp(frozen_ts_str)
    
    # Ensure we're working with a dict for transformations
    if isinstance(data, dict):
        working_data = data.copy()
    elif isinstance(data, list):
        # Wrap list in a dict temporarily for consistent processing
        working_data = {"_items": data}
        is_list_input = True
    else:
        raise TypeError(f"Expected dict or list, got {type(data)}")
    
    is_list_input = isinstance(data, list)
    
    # Apply deterministic transformations
    if deterministic:
        working_data = _apply_deterministic_properties(
            working_data,
            frozen_ts,
            float_precision,
        )
    else:
        # Inject variance token for non-deterministic mode
        if isinstance(data, dict):
            working_data = _inject_variance_token(working_data)
    
    # Unwrap if original input was a list
    if is_list_input:
        output_data = working_data["_items"]
    else:
        output_data = working_data
    
    # Serialize with optional key sorting
    json_str = json.dumps(
        output_data,
        indent=indent,
        sort_keys=stable_ids or deterministic,  # Always sort in deterministic mode
        ensure_ascii=False,
    )
    
    return json_str.encode("utf-8")
