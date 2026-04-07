"""Tests for JSON tree diff engine."""

import pytest

from ikam.diff.json_diff import compute_json_diff
from ikam.diff.types import DiffChange


def test_empty_dicts_no_changes():
    """Empty dicts should produce no changes."""
    result = compute_json_diff({}, {})
    assert result.change_count == 0
    assert result.affected_elements == 0
    assert len(result.changes) == 0


def test_identical_dicts_no_changes():
    """Identical dicts should produce no changes."""
    data = {"a": 1, "b": "hello", "c": [1, 2, 3]}
    result = compute_json_diff(data, data)
    assert result.change_count == 0


def test_added_top_level_key():
    """Added key at root level."""
    old = {"a": 1}
    new = {"a": 1, "b": 2}
    result = compute_json_diff(old, new)
    
    assert result.change_count == 1
    assert result.affected_elements == 1
    
    change = result.changes[0]
    assert change.path == "$.b"
    assert change.change_type == "added"
    assert change.old_value is None
    assert change.new_value == 2


def test_removed_top_level_key():
    """Removed key at root level."""
    old = {"a": 1, "b": 2}
    new = {"a": 1}
    result = compute_json_diff(old, new)
    
    assert result.change_count == 1
    change = result.changes[0]
    assert change.path == "$.b"
    assert change.change_type == "removed"
    assert change.old_value == 2
    assert change.new_value is None


def test_modified_top_level_value():
    """Modified value at root level."""
    old = {"a": 1}
    new = {"a": 2}
    result = compute_json_diff(old, new)
    
    assert result.change_count == 1
    change = result.changes[0]
    assert change.path == "$.a"
    assert change.change_type == "modified"
    assert change.old_value == 1
    assert change.new_value == 2


def test_nested_object_changes():
    """Changes in nested objects."""
    old = {"user": {"name": "Alice", "age": 30}}
    new = {"user": {"name": "Alice", "age": 31, "email": "alice@example.com"}}
    result = compute_json_diff(old, new)
    
    assert result.change_count == 2
    paths = {c.path for c in result.changes}
    assert "$.user.age" in paths
    assert "$.user.email" in paths
    
    # Find specific changes
    age_change = next(c for c in result.changes if c.path == "$.user.age")
    assert age_change.change_type == "modified"
    assert age_change.old_value == 30
    assert age_change.new_value == 31
    
    email_change = next(c for c in result.changes if c.path == "$.user.email")
    assert email_change.change_type == "added"
    assert email_change.new_value == "alice@example.com"


def test_array_element_modification():
    """Modified element in array."""
    old = {"items": [1, 2, 3]}
    new = {"items": [1, 5, 3]}
    result = compute_json_diff(old, new)
    
    assert result.change_count == 1
    change = result.changes[0]
    assert change.path == "$.items[1]"
    assert change.change_type == "modified"
    assert change.old_value == 2
    assert change.new_value == 5


def test_array_element_added():
    """Element added to array."""
    old = {"items": [1, 2]}
    new = {"items": [1, 2, 3]}
    result = compute_json_diff(old, new)
    
    assert result.change_count == 1
    change = result.changes[0]
    assert change.path == "$.items[2]"
    assert change.change_type == "added"
    assert change.new_value == 3


def test_array_element_removed():
    """Element removed from array."""
    old = {"items": [1, 2, 3]}
    new = {"items": [1, 2]}
    result = compute_json_diff(old, new)
    
    assert result.change_count == 1
    change = result.changes[0]
    assert change.path == "$.items[2]"
    assert change.change_type == "removed"
    assert change.old_value == 3


def test_nested_array_changes():
    """Changes in nested arrays."""
    old = {"data": [[1, 2], [3, 4]]}
    new = {"data": [[1, 5], [3, 4]]}
    result = compute_json_diff(old, new)
    
    assert result.change_count == 1
    change = result.changes[0]
    assert change.path == "$.data[0][1]"
    assert change.change_type == "modified"
    assert change.old_value == 2
    assert change.new_value == 5


def test_type_change_detected():
    """Type change is detected as modification."""
    old = {"value": 123}
    new = {"value": "123"}
    result = compute_json_diff(old, new)
    
    assert result.change_count == 1
    change = result.changes[0]
    assert change.change_type == "modified"
    assert change.old_value == 123
    assert change.new_value == "123"


def test_none_to_value():
    """None to value is treated as addition."""
    old = {"a": None}
    new = {"a": 1}
    result = compute_json_diff(old, new)
    
    # When old is None but key exists, it's treated as modified
    # When old is None and new has value at same path, _recursive_diff sees:
    # old_val=None, new_val=1 → added
    assert result.change_count == 1
    change = result.changes[0]
    assert change.path == "$.a"
    assert change.change_type == "added"


def test_value_to_none():
    """Value to None is treated as removal."""
    old = {"a": 1}
    new = {"a": None}
    result = compute_json_diff(old, new)
    
    assert result.change_count == 1
    change = result.changes[0]
    assert change.path == "$.a"
    assert change.change_type == "removed"


def test_complex_nested_structure():
    """Complex nested structure with multiple changes."""
    old = {
        "meta": {"version": 1, "author": "Alice"},
        "data": {
            "users": [
                {"id": 1, "name": "Bob"},
                {"id": 2, "name": "Charlie"},
            ],
            "settings": {"theme": "dark"},
        },
    }
    new = {
        "meta": {"version": 2, "author": "Alice"},
        "data": {
            "users": [
                {"id": 1, "name": "Bob", "email": "bob@example.com"},
                {"id": 2, "name": "Charles"},  # name changed
            ],
            "settings": {"theme": "light", "locale": "en"},
        },
    }
    result = compute_json_diff(old, new)
    
    # Changes:
    # 1. $.meta.version: 1 -> 2
    # 2. $.data.users[0].email: added
    # 3. $.data.users[1].name: Charlie -> Charles
    # 4. $.data.settings.theme: dark -> light
    # 5. $.data.settings.locale: added
    assert result.change_count == 5
    
    paths = {c.path for c in result.changes}
    assert "$.meta.version" in paths
    assert "$.data.users[0].email" in paths
    assert "$.data.users[1].name" in paths
    assert "$.data.settings.theme" in paths
    assert "$.data.settings.locale" in paths


def test_empty_to_populated():
    """Empty dict to populated dict."""
    old = {}
    new = {"a": 1, "b": 2}
    result = compute_json_diff(old, new)
    
    assert result.change_count == 2
    assert all(c.change_type == "added" for c in result.changes)


def test_populated_to_empty():
    """Populated dict to empty dict."""
    old = {"a": 1, "b": 2}
    new = {}
    result = compute_json_diff(old, new)
    
    assert result.change_count == 2
    assert all(c.change_type == "removed" for c in result.changes)


def test_list_input_top_level():
    """Top-level list comparison."""
    old = [1, 2, 3]
    new = [1, 5, 3, 4]
    result = compute_json_diff(old, new)
    
    # Changes: $[1] modified (2->5), $[3] added
    assert result.change_count == 2
    paths = {c.path for c in result.changes}
    assert "$[1]" in paths
    assert "$[3]" in paths


def test_affected_elements_count():
    """Affected elements count is distinct paths."""
    old = {"a": {"b": 1, "c": 2}}
    new = {"a": {"b": 5, "c": 6}}
    result = compute_json_diff(old, new)
    
    # 2 changes, 2 distinct paths
    assert result.change_count == 2
    assert result.affected_elements == 2


def test_diff_change_validation():
    """DiffChange validates invariants."""
    # Valid added change
    DiffChange(path="$.a", change_type="added", new_value=1)
    
    # Valid removed change
    DiffChange(path="$.a", change_type="removed", old_value=1)
    
    # Valid modified change
    DiffChange(path="$.a", change_type="modified", old_value=1, new_value=2)
    
    # Invalid: added with old_value
    with pytest.raises(ValueError, match="Added changes must have old_value=None"):
        DiffChange(path="$.a", change_type="added", old_value=1, new_value=2)
    
    # Invalid: removed with new_value
    with pytest.raises(ValueError, match="Removed changes must have new_value=None"):
        DiffChange(path="$.a", change_type="removed", old_value=1, new_value=2)
    
    # Invalid: modified without old_value
    with pytest.raises(ValueError, match="Modified changes must have both"):
        DiffChange(path="$.a", change_type="modified", new_value=2)
