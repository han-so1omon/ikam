"""
Tests for deterministic JSON rendering.

Validates that IKAM_* environment variables control JSON output determinism.
"""

import hashlib
import json
import pytest
from ikam.renderers.json import render_json


def _hash_bytes(data: bytes) -> str:
    """Compute blake2b hash of bytes."""
    return hashlib.blake2b(data).hexdigest()


def test_non_deterministic_json_differs(monkeypatch):
    """Non-deterministic mode should produce different outputs on repeated renders."""
    # Clear all deterministic env vars to ensure non-deterministic mode
    monkeypatch.delenv("IKAM_DETERMINISTIC_RENDER", raising=False)
    monkeypatch.delenv("IKAM_FROZEN_TIMESTAMP", raising=False)
    monkeypatch.delenv("IKAM_STABLE_IDS", raising=False)
    monkeypatch.delenv("IKAM_FLOAT_PRECISION", raising=False)
    
    data = {
        "name": "Test Document",
        "value": 3.14159265359,
        "created_at": "2024-01-01T00:00:00Z",
    }
    
    render1 = render_json(data)
    render2 = render_json(data)
    
    # Hashes should differ due to variance token
    hash1 = _hash_bytes(render1)
    hash2 = _hash_bytes(render2)
    assert hash1 != hash2, "Non-deterministic renders should produce different hashes"
    
    # But both should be valid JSON
    parsed1 = json.loads(render1)
    parsed2 = json.loads(render2)
    assert "_variance_token" in parsed1
    assert "_variance_token" in parsed2
    assert parsed1["_variance_token"] != parsed2["_variance_token"]


def test_deterministic_json_stable(monkeypatch):
    """Deterministic mode should produce identical outputs on repeated renders."""
    monkeypatch.setenv("IKAM_DETERMINISTIC_RENDER", "true")
    monkeypatch.setenv("IKAM_FROZEN_TIMESTAMP", "2024-01-01T00:00:00Z")
    monkeypatch.setenv("IKAM_FLOAT_PRECISION", "6")
    
    data = {
        "name": "Test Document",
        "value": 3.14159265359,
        "created_at": "2024-11-30T12:34:56Z",
    }
    
    render1 = render_json(data)
    render2 = render_json(data)
    
    # Hashes should be identical
    hash1 = _hash_bytes(render1)
    hash2 = _hash_bytes(render2)
    assert hash1 == hash2, "Deterministic renders should produce identical hashes"
    
    # Verify transformations applied
    parsed = json.loads(render1)
    assert parsed["created_at"] == "2024-01-01T00:00:00Z"  # Frozen timestamp
    assert parsed["value"] == 3.141593  # Rounded to 6 decimals
    assert "_variance_token" not in parsed


def test_key_sorting_with_stable_ids(monkeypatch):
    """IKAM_STABLE_IDS=true should sort keys lexicographically."""
    monkeypatch.setenv("IKAM_STABLE_IDS", "true")
    
    data = {
        "zebra": 1,
        "apple": 2,
        "middle": 3,
    }
    
    output = render_json(data)
    parsed = json.loads(output)
    
    # Keys should be in sorted order
    keys = list(parsed.keys())
    # Filter out variance token if present
    data_keys = [k for k in keys if k != "_variance_token"]
    assert data_keys == ["apple", "middle", "zebra"]


def test_float_precision_rounding(monkeypatch):
    """IKAM_FLOAT_PRECISION should round all floats to specified decimals."""
    monkeypatch.setenv("IKAM_DETERMINISTIC_RENDER", "true")
    monkeypatch.setenv("IKAM_FLOAT_PRECISION", "2")
    
    data = {
        "pi": 3.14159265359,
        "nested": {
            "value": 2.71828182846,
        },
        "list": [1.414213562, 1.732050808],
    }
    
    output = render_json(data)
    parsed = json.loads(output)
    
    assert parsed["pi"] == 3.14
    assert parsed["nested"]["value"] == 2.72
    assert parsed["list"] == [1.41, 1.73]


def test_list_input():
    """Renderer should handle list input correctly."""
    data = [
        {"id": 1, "name": "First"},
        {"id": 2, "name": "Second"},
    ]
    
    output = render_json(data)
    parsed = json.loads(output)
    
    assert isinstance(parsed, list)
    assert len(parsed) == 2
    assert parsed[0]["id"] == 1
    assert parsed[1]["id"] == 2


def test_deterministic_list_stable(monkeypatch):
    """Deterministic mode should work with list inputs."""
    monkeypatch.setenv("IKAM_DETERMINISTIC_RENDER", "true")
    monkeypatch.setenv("IKAM_FROZEN_TIMESTAMP", "2024-01-01T00:00:00Z")
    
    data = [
        {"timestamp": "2024-11-30T12:00:00Z", "value": 1.5},
        {"timestamp": "2024-11-30T13:00:00Z", "value": 2.5},
    ]
    
    render1 = render_json(data)
    render2 = render_json(data)
    
    # Should be byte-identical
    assert render1 == render2
    
    # Verify frozen timestamps
    parsed = json.loads(render1)
    assert parsed[0]["timestamp"] == "2024-01-01T00:00:00Z"
    assert parsed[1]["timestamp"] == "2024-01-01T00:00:00Z"


def test_invalid_timestamp_raises(monkeypatch):
    """Invalid IKAM_FROZEN_TIMESTAMP should raise ValueError."""
    monkeypatch.setenv("IKAM_FROZEN_TIMESTAMP", "not-a-timestamp")
    
    data = {"key": "value"}
    
    with pytest.raises(ValueError, match="Invalid timestamp format"):
        render_json(data)


def test_nested_timestamp_freezing(monkeypatch):
    """Frozen timestamp should apply to nested timestamp fields."""
    monkeypatch.setenv("IKAM_DETERMINISTIC_RENDER", "true")
    monkeypatch.setenv("IKAM_FROZEN_TIMESTAMP", "2024-01-01T00:00:00Z")
    
    data = {
        "created_at": "2024-11-30T10:00:00Z",
        "sections": [
            {
                "updated_at": "2024-11-30T11:00:00Z",
                "metadata": {
                    "modified": "2024-11-30T12:00:00Z",
                }
            }
        ]
    }
    
    output = render_json(data)
    parsed = json.loads(output)
    
    assert parsed["created_at"] == "2024-01-01T00:00:00Z"
    assert parsed["sections"][0]["updated_at"] == "2024-01-01T00:00:00Z"
    assert parsed["sections"][0]["metadata"]["modified"] == "2024-01-01T00:00:00Z"
