"""JSON tree diff engine with recursive comparison.

Implements deep structural comparison of JSON trees with JSONPath notation
for change paths. Supports nested objects, arrays, and primitive types.

Performance: O(N) where N is total number of leaf nodes in both trees.
"""

from typing import Any, Dict, List, Union

from .types import DiffChange, DiffResult


def compute_json_diff(
    old_data: Union[Dict[str, Any], List[Any], None],
    new_data: Union[Dict[str, Any], List[Any], None],
) -> DiffResult:
    """Compute structural diff between two JSON documents.
    
    Args:
        old_data: Original JSON data (dict, list, or None)
        new_data: Updated JSON data (dict, list, or None)
        
    Returns:
        DiffResult with list of changes, counts, and metadata
        
    Examples:
        >>> old = {"a": 1, "b": {"c": 2}}
        >>> new = {"a": 1, "b": {"c": 3, "d": 4}}
        >>> result = compute_json_diff(old, new)
        >>> len(result.changes)
        2
        >>> result.changes[0].path
        '$.b.c'
        >>> result.changes[1].path
        '$.b.d'
    """
    changes: List[DiffChange] = []
    _recursive_diff(old_data, new_data, "$", changes)
    
    return DiffResult(
        changes=changes,
        change_count=len(changes),
        affected_elements=len(set(c.path for c in changes)),
    )


def _recursive_diff(
    old_val: Any,
    new_val: Any,
    path: str,
    changes: List[DiffChange],
) -> None:
    """Recursively compare two values and record changes.
    
    Args:
        old_val: Original value at this path
        new_val: Updated value at this path
        path: Current JSONPath (e.g., "$.a.b[0]")
        changes: Accumulator for discovered changes
    """
    # Handle None cases
    if old_val is None and new_val is None:
        return
    if old_val is None:
        changes.append(DiffChange(path=path, change_type="added", new_value=new_val))
        return
    if new_val is None:
        changes.append(DiffChange(path=path, change_type="removed", old_value=old_val))
        return
    
    # Type mismatch is a modification
    if type(old_val) != type(new_val):
        changes.append(DiffChange(
            path=path,
            change_type="modified",
            old_value=old_val,
            new_value=new_val,
        ))
        return
    
    # Recursive comparison for dicts
    if isinstance(old_val, dict) and isinstance(new_val, dict):
        _diff_dicts(old_val, new_val, path, changes)
        return
    
    # Recursive comparison for lists
    if isinstance(old_val, list) and isinstance(new_val, list):
        _diff_lists(old_val, new_val, path, changes)
        return
    
    # Primitive comparison
    if old_val != new_val:
        changes.append(DiffChange(
            path=path,
            change_type="modified",
            old_value=old_val,
            new_value=new_val,
        ))


def _diff_dicts(
    old_dict: Dict[str, Any],
    new_dict: Dict[str, Any],
    path: str,
    changes: List[DiffChange],
) -> None:
    """Compare two dictionaries recursively."""
    old_keys = set(old_dict.keys())
    new_keys = set(new_dict.keys())
    
    # Added keys
    for key in new_keys - old_keys:
        child_path = f"{path}.{key}"
        changes.append(DiffChange(
            path=child_path,
            change_type="added",
            new_value=new_dict[key],
        ))
    
    # Removed keys
    for key in old_keys - new_keys:
        child_path = f"{path}.{key}"
        changes.append(DiffChange(
            path=child_path,
            change_type="removed",
            old_value=old_dict[key],
        ))
    
    # Common keys - recurse
    for key in old_keys & new_keys:
        child_path = f"{path}.{key}"
        _recursive_diff(old_dict[key], new_dict[key], child_path, changes)


def _diff_lists(
    old_list: List[Any],
    new_list: List[Any],
    path: str,
    changes: List[DiffChange],
) -> None:
    """Compare two lists by index.
    
    Note: This is a simple index-based comparison. For MVP, we don't implement
    LCS/diff algorithms for optimal list diffs. Arrays are compared element-by-element
    up to min(len(old), len(new)), then remaining elements are added/removed.
    """
    min_len = min(len(old_list), len(new_list))
    
    # Compare common indices
    for i in range(min_len):
        child_path = f"{path}[{i}]"
        _recursive_diff(old_list[i], new_list[i], child_path, changes)
    
    # Handle length differences
    if len(new_list) > len(old_list):
        # Elements added to new list
        for i in range(min_len, len(new_list)):
            child_path = f"{path}[{i}]"
            changes.append(DiffChange(
                path=child_path,
                change_type="added",
                new_value=new_list[i],
            ))
    elif len(old_list) > len(new_list):
        # Elements removed from old list
        for i in range(min_len, len(old_list)):
            child_path = f"{path}[{i}]"
            changes.append(DiffChange(
                path=child_path,
                change_type="removed",
                old_value=old_list[i],
            ))
