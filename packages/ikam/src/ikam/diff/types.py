"""Common types for diff engines."""

from dataclasses import dataclass
from typing import Any, List, Literal


@dataclass
class DiffChange:
    """Individual change in a diff result."""
    path: str  # JSONPath ($.a.b[0]) or cell reference (Sheet1!A1)
    change_type: Literal["added", "removed", "modified"]
    old_value: Any = None
    new_value: Any = None
    
    def __post_init__(self):
        """Validate change invariants."""
        if self.change_type == "added" and self.old_value is not None:
            raise ValueError("Added changes must have old_value=None")
        if self.change_type == "removed" and self.new_value is not None:
            raise ValueError("Removed changes must have new_value=None")
        if self.change_type == "modified" and (self.old_value is None or self.new_value is None):
            raise ValueError("Modified changes must have both old_value and new_value")


@dataclass
class DiffResult:
    """Result of a diff computation."""
    changes: List[DiffChange]
    change_count: int
    affected_elements: int  # Distinct paths affected
    
    def __post_init__(self):
        """Compute derived fields."""
        if self.change_count == 0:
            self.change_count = len(self.changes)
        if self.affected_elements == 0:
            # Count distinct paths
            self.affected_elements = len(set(c.path for c in self.changes))
