"""
Tests for LCS/Myers array diff algorithm.

Validates optimal sequence comparison with move detection.
"""
import pytest
from ikam.diff.array_diff import (
    diff_arrays,
    diff_arrays_simple,
    array_edit_distance,
    ArrayOperation,
    _compute_lcs_length,
    _detect_moves,
)


# Test: Identical arrays

def test_diff_arrays_identical():
    """Identical arrays should return only unchanged operations."""
    arr = [1, 2, 3, 4, 5]
    
    operations = diff_arrays(arr, arr)
    
    assert len(operations) == 5
    assert all(op.operation == "unchanged" for op in operations)


# Test: Simple additions

def test_diff_arrays_additions():
    """Detect elements added to end of array."""
    old = [1, 2, 3]
    new = [1, 2, 3, 4, 5]
    
    operations = diff_arrays(old, new)
    
    added = [op for op in operations if op.operation == "add"]
    assert len(added) == 2
    assert added[0].element == 4
    assert added[1].element == 5
    assert added[0].new_index == 3
    assert added[1].new_index == 4


# Test: Simple removals

def test_diff_arrays_removals():
    """Detect elements removed from array."""
    old = [1, 2, 3, 4, 5]
    new = [1, 3, 5]
    
    operations = diff_arrays(old, new)
    
    removed = [op for op in operations if op.operation == "remove"]
    assert len(removed) == 2
    assert removed[0].element == 2
    assert removed[1].element == 4


# Test: Element moves

def test_diff_arrays_moves():
    """Detect element reordering as moves."""
    old = [1, 2, 3]
    new = [3, 1, 2]
    
    operations = diff_arrays(old, new, detect_moves=True)
    
    moves = [op for op in operations if op.operation == "move"]
    # All elements moved to different positions
    assert len(moves) >= 1  # At least one move detected


def test_diff_arrays_swap():
    """Detect simple swap as moves."""
    old = [1, 2, 3, 4]
    new = [1, 3, 2, 4]  # Swap 2 and 3
    
    operations = diff_arrays(old, new, detect_moves=True)
    
    moves = [op for op in operations if op.operation == "move"]
    unchanged = [op for op in operations if op.operation == "unchanged"]
    
    # Elements 1 and 4 should be unchanged
    assert len(unchanged) >= 2
    # Elements 2 and 3 should be detected as moved
    assert len(moves) >= 1


# Test: Mixed operations

def test_diff_arrays_mixed():
    """Handle mix of adds, removes, and unchanged."""
    old = [1, 2, 3, 4]
    new = [1, 5, 3, 6]  # Keep 1,3; remove 2,4; add 5,6
    
    operations = diff_arrays(old, new, detect_moves=False)
    
    added = [op for op in operations if op.operation == "add"]
    removed = [op for op in operations if op.operation == "remove"]
    unchanged = [op for op in operations if op.operation == "unchanged"]
    
    assert len(added) == 2
    assert len(removed) == 2
    assert len(unchanged) == 2
    
    assert set(op.element for op in added) == {5, 6}
    assert set(op.element for op in removed) == {2, 4}
    assert set(op.element for op in unchanged) == {1, 3}


# Test: Empty arrays

def test_diff_arrays_empty_to_content():
    """Adding elements to empty array."""
    old = []
    new = [1, 2, 3]
    
    operations = diff_arrays(old, new)
    
    assert len(operations) == 3
    assert all(op.operation == "add" for op in operations)


def test_diff_arrays_content_to_empty():
    """Removing all elements."""
    old = [1, 2, 3]
    new = []
    
    operations = diff_arrays(old, new)
    
    assert len(operations) == 3
    assert all(op.operation == "remove" for op in operations)


def test_diff_arrays_both_empty():
    """Both arrays empty."""
    operations = diff_arrays([], [])
    assert len(operations) == 0


# Test: Custom equality function

def test_diff_arrays_custom_equality():
    """Use custom equality for object comparison."""
    
    class Item:
        def __init__(self, id, value):
            self.id = id
            self.value = value
        
        def __repr__(self):
            return f"Item({self.id}, {self.value})"
    
    # Equality based on ID only
    def equals_by_id(a, b):
        return a.id == b.id
    
    old = [Item(1, "a"), Item(2, "b"), Item(3, "c")]
    new = [Item(1, "x"), Item(2, "y"), Item(4, "d")]  # Changed values + new item
    
    operations = diff_arrays(old, new, equals=equals_by_id)
    
    # Items 1 and 2 match by ID (values don't matter)
    unchanged = [op for op in operations if op.operation == "unchanged"]
    assert len(unchanged) == 2
    
    # Item 3 removed, Item 4 added
    removed = [op for op in operations if op.operation == "remove"]
    added = [op for op in operations if op.operation == "add"]
    assert len(removed) == 1
    assert len(added) == 1


# Test: Simple interface

def test_diff_arrays_simple_interface():
    """Test simplified interface returning element lists."""
    old = [1, 2, 3, 4]
    new = [2, 3, 5, 6]
    
    added, removed, unchanged = diff_arrays_simple(old, new)
    
    assert set(added) == {5, 6}
    assert set(removed) == {1, 4}
    assert set(unchanged) == {2, 3}


# Test: Edit distance

def test_array_edit_distance_identical():
    """Edit distance for identical arrays is 0."""
    arr = [1, 2, 3]
    assert array_edit_distance(arr, arr) == 0


def test_array_edit_distance_complete_replacement():
    """Edit distance when all elements differ."""
    old = [1, 2]
    new = [3, 4]
    
    # Need to remove 1,2 and add 3,4 = 4 operations
    assert array_edit_distance(old, new) == 4


def test_array_edit_distance_partial():
    """Edit distance with some overlap."""
    old = [1, 2, 3]
    new = [2, 3, 4]
    
    # Remove 1, add 4 = 2 operations (2 and 3 are in LCS)
    assert array_edit_distance(old, new) == 2


# Test: LCS computation

def test_compute_lcs_length():
    """Verify LCS length matrix computation."""
    old = [1, 2, 3]
    new = [1, 3, 2]
    
    dp = _compute_lcs_length(old, new)
    
    # LCS length should be 2 (either "1,2" or "1,3")
    assert dp[len(old)][len(new)] == 2


# Test: Move detection

def test_detect_moves_basic():
    """Test move detection on basic operations."""
    # Simulate: [1, 2] -> [2, 1]
    # Remove 1 at index 0, add 1 at index 1
    # Remove 2 at index 1, add 2 at index 0
    operations = [
        ArrayOperation(operation="remove", element=1, old_index=0, new_index=None),
        ArrayOperation(operation="add", element=2, old_index=None, new_index=0),
        ArrayOperation(operation="remove", element=2, old_index=1, new_index=None),
        ArrayOperation(operation="add", element=1, old_index=None, new_index=1),
    ]
    
    result = _detect_moves(operations)
    
    # Should convert remove+add pairs into moves
    moves = [op for op in result if op.operation == "move"]
    assert len(moves) == 2


# Test: ArrayOperation validation

def test_array_operation_add_validation():
    """Add operations must have old_index=None."""
    with pytest.raises(ValueError, match="Add operations must have old_index=None"):
        ArrayOperation(operation="add", element=1, old_index=0, new_index=1)


def test_array_operation_remove_validation():
    """Remove operations must have new_index=None."""
    with pytest.raises(ValueError, match="Remove operations must have new_index=None"):
        ArrayOperation(operation="remove", element=1, old_index=0, new_index=1)


def test_array_operation_move_validation():
    """Move operations must have both indices."""
    with pytest.raises(ValueError, match="Move operations must have both"):
        ArrayOperation(operation="move", element=1, old_index=0, new_index=None)


# Test: Large array performance (basic smoke test)

def test_diff_arrays_large():
    """Ensure reasonable performance on larger arrays."""
    import time
    
    # Generate 500-element arrays with 10% difference
    old = list(range(500))
    new = list(range(50, 550))  # Shift by 50
    
    start = time.perf_counter()
    operations = diff_arrays(old, new)
    elapsed = time.perf_counter() - start
    
    # Should complete in reasonable time (<100ms target)
    assert elapsed < 0.5, f"Took {elapsed*1000:.1f}ms (expected <500ms)"
    
    # Verify correctness
    assert len(operations) > 0
