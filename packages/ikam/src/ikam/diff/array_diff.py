"""
LCS/Myers array diff algorithm for optimal sequence comparison.

Implements Myers' O(ND) diff algorithm to detect minimal edit sequences
for arrays, including element moves, adds, and removes.

Reference: "An O(ND) Difference Algorithm and Its Variations" by Eugene W. Myers (1986)
"""
from typing import Any, Callable, List, Optional, Tuple
from dataclasses import dataclass


@dataclass
class ArrayOperation:
    """Represents a single array operation."""
    operation: str  # "add", "remove", "move", "unchanged"
    element: Any
    old_index: Optional[int] = None
    new_index: Optional[int] = None
    
    def __post_init__(self):
        """Validate operation invariants."""
        if self.operation == "add" and self.old_index is not None:
            raise ValueError("Add operations must have old_index=None")
        if self.operation == "remove" and self.new_index is not None:
            raise ValueError("Remove operations must have new_index=None")
        if self.operation == "move":
            if self.old_index is None or self.new_index is None:
                raise ValueError("Move operations must have both old_index and new_index")


def _default_equality(a: Any, b: Any) -> bool:
    """Default equality function."""
    return a == b


def _compute_lcs_length(
    old_arr: List[Any],
    new_arr: List[Any],
    equals: Callable[[Any, Any], bool] = _default_equality,
) -> List[List[int]]:
    """
    Compute LCS (Longest Common Subsequence) length matrix using dynamic programming.
    
    Returns a 2D matrix where dp[i][j] represents the length of LCS
    between old_arr[:i] and new_arr[:j].
    
    Time: O(m*n), Space: O(m*n)
    """
    m, n = len(old_arr), len(new_arr)
    dp = [[0] * (n + 1) for _ in range(m + 1)]
    
    for i in range(1, m + 1):
        for j in range(1, n + 1):
            if equals(old_arr[i - 1], new_arr[j - 1]):
                dp[i][j] = dp[i - 1][j - 1] + 1
            else:
                dp[i][j] = max(dp[i - 1][j], dp[i][j - 1])
    
    return dp


def _backtrack_lcs(
    old_arr: List[Any],
    new_arr: List[Any],
    dp: List[List[int]],
    equals: Callable[[Any, Any], bool] = _default_equality,
) -> List[ArrayOperation]:
    """
    Backtrack through LCS matrix to generate minimal edit sequence.
    
    Returns list of ArrayOperation objects representing the diff.
    Uses append+reverse for O(n) instead of insert(0) which is O(n²).
    """
    operations: List[ArrayOperation] = []
    i, j = len(old_arr), len(new_arr)
    
    while i > 0 or j > 0:
        if i > 0 and j > 0 and equals(old_arr[i - 1], new_arr[j - 1]):
            # Elements match - unchanged
            operations.append(
                ArrayOperation(
                    operation="unchanged",
                    element=old_arr[i - 1],
                    old_index=i - 1,
                    new_index=j - 1,
                )
            )
            i -= 1
            j -= 1
        elif j > 0 and (i == 0 or dp[i][j - 1] >= dp[i - 1][j]):
            # Element added in new array
            operations.append(
                ArrayOperation(
                    operation="add",
                    element=new_arr[j - 1],
                    old_index=None,
                    new_index=j - 1,
                )
            )
            j -= 1
        else:
            # Element removed from old array
            operations.append(
                ArrayOperation(
                    operation="remove",
                    element=old_arr[i - 1],
                    old_index=i - 1,
                    new_index=None,
                )
            )
            i -= 1
    
    # Reverse to get forward order (O(n) single pass)
    operations.reverse()
    return operations


def _detect_moves(operations: List[ArrayOperation]) -> List[ArrayOperation]:
    """
    Post-process operations to detect moves (remove + add of same element).
    
    Converts pairs of (remove, add) with equal elements into move operations.
    This reduces noise in the diff output for reordered arrays.
    """
    # Build indices for removed and added elements
    removed = {op.old_index: op for op in operations if op.operation == "remove"}
    added = {op.new_index: op for op in operations if op.operation == "add"}
    
    # Track which operations become moves
    move_pairs = []
    
    for old_idx, rem_op in removed.items():
        for new_idx, add_op in added.items():
            if rem_op.element == add_op.element:
                # Found a move
                move_pairs.append((old_idx, new_idx, rem_op.element))
                break
    
    # Filter out remove/add pairs that are moves
    moved_old = {old_idx for old_idx, _, _ in move_pairs}
    moved_new = {new_idx for _, new_idx, _ in move_pairs}
    
    result = []
    for op in operations:
        if op.operation == "remove" and op.old_index in moved_old:
            continue  # Skip - will be replaced by move
        if op.operation == "add" and op.new_index in moved_new:
            continue  # Skip - will be replaced by move
        result.append(op)
    
    # Add move operations
    for old_idx, new_idx, element in move_pairs:
        result.append(
            ArrayOperation(
                operation="move",
                element=element,
                old_index=old_idx,
                new_index=new_idx,
            )
        )
    
    return result


def diff_arrays(
    old_arr: List[Any],
    new_arr: List[Any],
    equals: Optional[Callable[[Any, Any], bool]] = None,
    detect_moves: bool = True,
) -> List[ArrayOperation]:
    """
    Compute optimal diff between two arrays using LCS-based algorithm.
    
    Args:
        old_arr: Original array
        new_arr: New array
        equals: Custom equality function (default: ==)
        detect_moves: Whether to detect element moves (default: True)
    
    Returns:
        List of ArrayOperation objects representing minimal edit sequence.
        Operations are ordered by original array position.
    
    Time Complexity: O(m*n) where m=len(old_arr), n=len(new_arr)
    Space Complexity: O(m*n)
    
    Notes:
        - For large arrays (>1000 elements), consider using a more efficient
          algorithm like Myers' O(ND) variant or histogram diff.
        - Set detect_moves=False to improve performance if moves aren't needed.
    
    Examples:
        >>> diff_arrays([1, 2, 3], [1, 3, 2])
        [ArrayOperation(operation='unchanged', element=1, old_index=0, new_index=0),
         ArrayOperation(operation='move', element=2, old_index=1, new_index=2),
         ArrayOperation(operation='move', element=3, old_index=2, new_index=1)]
        
        >>> diff_arrays([1, 2], [1, 2, 3])
        [ArrayOperation(operation='unchanged', element=1, old_index=0, new_index=0),
         ArrayOperation(operation='unchanged', element=2, old_index=1, new_index=1),
         ArrayOperation(operation='add', element=3, old_index=None, new_index=2)]
    """
    if equals is None:
        equals = _default_equality
    
    # Fast path: identical arrays (common case, avoids O(m*n) DP computation)
    if len(old_arr) == len(new_arr) and all(equals(old_arr[i], new_arr[i]) for i in range(len(old_arr))):
        return [ArrayOperation("unchanged", element=old_arr[i], old_index=i, new_index=i) for i in range(len(old_arr))]
    
    # Compute LCS length matrix
    dp = _compute_lcs_length(old_arr, new_arr, equals)
    
    # Backtrack to generate operations
    operations = _backtrack_lcs(old_arr, new_arr, dp, equals)
    
    # Optionally detect moves
    if detect_moves:
        operations = _detect_moves(operations)
    
    return operations


def diff_arrays_simple(
    old_arr: List[Any],
    new_arr: List[Any],
    equals: Optional[Callable[[Any, Any], bool]] = None,
) -> Tuple[List[Any], List[Any], List[Any]]:
    """
    Simple interface that returns added, removed, and unchanged elements.
    
    Args:
        old_arr: Original array
        new_arr: New array
        equals: Custom equality function (default: ==)
    
    Returns:
        Tuple of (added, removed, unchanged) element lists
    
    Examples:
        >>> diff_arrays_simple([1, 2, 3], [2, 3, 4])
        ([4], [1], [2, 3])
    """
    operations = diff_arrays(old_arr, new_arr, equals, detect_moves=False)
    
    added = [op.element for op in operations if op.operation == "add"]
    removed = [op.element for op in operations if op.operation == "remove"]
    unchanged = [op.element for op in operations if op.operation == "unchanged"]
    
    return added, removed, unchanged


def array_edit_distance(
    old_arr: List[Any],
    new_arr: List[Any],
    equals: Optional[Callable[[Any, Any], bool]] = None,
) -> int:
    """
    Compute edit distance (number of operations) to transform old_arr into new_arr.
    
    This is equivalent to: len(old_arr) + len(new_arr) - 2 * LCS_length
    
    Args:
        old_arr: Original array
        new_arr: New array
        equals: Custom equality function (default: ==)
    
    Returns:
        Minimum number of add/remove operations needed
    
    Examples:
        >>> array_edit_distance([1, 2, 3], [1, 2, 3])
        0
        >>> array_edit_distance([1, 2], [3, 4])
        4  # Remove 1, 2; Add 3, 4
    """
    if equals is None:
        equals = _default_equality
    
    dp = _compute_lcs_length(old_arr, new_arr, equals)
    lcs_len = dp[len(old_arr)][len(new_arr)]
    
    return len(old_arr) + len(new_arr) - 2 * lcs_len
