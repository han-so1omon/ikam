"""Diff engines for IKAM artifact comparison.

Provides format-specific diff implementations:
- JSON: Deep tree comparison with JSONPath notation
- XLSX: Cell-level comparison with sheet/cell references, formula tracking
- PPTX: Slide-level comparison with fingerprint-based change detection
- Arrays: LCS-based optimal diff with move detection
"""

from .array_diff import diff_arrays, diff_arrays_simple, array_edit_distance
from .json_diff import compute_json_diff
from .pptx_diff import diff_pptx, diff_pptx_files
from .types import DiffChange, DiffResult
from .xlsx_diff import compute_xlsx_diff

__all__ = [
    "compute_json_diff",
    "compute_xlsx_diff",
    "diff_pptx",
    "diff_pptx_files",
    "diff_arrays",
    "diff_arrays_simple",
    "array_edit_distance",
    "DiffChange",
    "DiffResult",
]
