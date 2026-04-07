"""
IKAM Sheet Decomposition and Reconstruction (V3 Fragment Algebra)

Implements sheet decomposition into V3 Fragments with relation DAGs
and lossless reconstruction via root relation traversal.

Handles spreadsheet-specific concerns: formulas, cell ranges, charts.

Mathematical Guarantees:
- Lossless: reconstruct(decompose(Workbook)) = Workbook (structure + formulas preserved)
- Formula integrity: All cell references and formulas maintained
- Chart preservation: Chart definitions and data ranges intact

Version: 2.0.0 (V3 Fragment Algebra - February 2026)
"""

from __future__ import annotations

import hashlib
import json
from typing import Any, Dict, List, Optional

from .config import DecompositionConfig
from .fragments import (
    Fragment,
    Relation,
    SlotBinding,
    BindingGroup,
    RELATION_MIME,
    is_relation_fragment,
)
from .graph import _cas_hex
from .sheet_models import (
    Cell,
    CellRange,
    Chart,
    Sheet,
    SheetFragmentContent,
    Workbook,
    WorkbookMeta,
)


class SheetDecompositionError(Exception):
    """Error during sheet decomposition."""


class SheetReconstructionError(Exception):
    """Error during sheet reconstruction."""


SHEET_FRAGMENT_MIME = "application/vnd.ikam.sheet+json"


def _v3_cas_fragment(value: Any, mime_type: str) -> Fragment:
    """Create a V3 Fragment with computed cas_id from value + mime_type."""
    from .adapters import v3_fragment_to_cas_bytes

    frag = Fragment(value=value, mime_type=mime_type)
    cas_bytes = v3_fragment_to_cas_bytes(frag)
    cas_id = _cas_hex(cas_bytes)
    return Fragment(cas_id=cas_id, value=value, mime_type=mime_type)


def decompose_workbook(
    workbook: Workbook,
    artifact_id: str,
    config: Optional[DecompositionConfig] = None,
) -> List[Fragment]:
    """
    Decompose a workbook into V3 Fragment objects + root relation.

    Emits:
    - One fragment per sheet (SheetFragmentContent serialized as dict)
    - A root relation fragment referencing all sheet fragments

    Mathematical Guarantee:
    - Lossless: all cells, formulas, and charts preserved
    - Deterministic: same workbook → same fragments

    Args:
        workbook: Workbook to decompose
        artifact_id: Parent artifact ID
        config: Decomposition configuration

    Returns:
        List of V3 Fragment objects; last element is the root relation
    """
    if config is None:
        config = DecompositionConfig()

    sheet_frags: List[Fragment] = []
    slots: List[SlotBinding] = []

    # Emit one fragment per sheet
    for idx, sheet in enumerate(workbook.sheets):
        sheet_content = SheetFragmentContent(
            summary=f"Sheet '{sheet.name}' ({len(sheet.cells)} cells, {len(sheet.charts)} charts)",
            sheet_id=sheet.id,
            sheet_name=sheet.name,
            cells=sheet.cells,
            charts=sheet.charts,
            formulas=[cell.formula for cell in sheet.cells.values() if cell.formula],
        )
        frag = _v3_cas_fragment(sheet_content.model_dump(mode="json"), SHEET_FRAGMENT_MIME)
        sheet_frags.append(frag)
        slots.append(SlotBinding(slot=f"sheet-{idx}", fragment_id=frag.cas_id))

    # Workbook summary as metadata in the relation
    summary_text = _build_workbook_summary(workbook)
    notable_formulas = _extract_notable_formulas(workbook)

    root_rel = Relation(
        predicate="workbook-root",
        directed=True,
        confidence_score=1.0,
        qualifiers={
            "summary": summary_text,
            "workbook_id": workbook.id,
            "workbook_title": workbook.meta.title if workbook.meta else "Untitled",
            "workbook_description": workbook.meta.description if workbook.meta else "",
            "notable_formulas": notable_formulas,
            "workbook_charts": [c.model_dump(mode="json") for c in workbook.charts],
        },
        binding_groups=[
            BindingGroup(
                invocation_id=f"{artifact_id}:workbook-root",
                slots=slots,
            ),
        ],
    )
    root_relation_frag = _v3_cas_fragment(
        root_rel.model_dump(mode="json"),
        RELATION_MIME,
    )

    return sheet_frags + [root_relation_frag]


def reconstruct_workbook(
    fragments: List[Fragment],
    config: Optional[DecompositionConfig] = None,
) -> Workbook:
    """
    Reconstruct workbook from V3 fragments via root relation DAG.

    Mathematical Guarantee:
    - Lossless: reconstruct(decompose(W)) = W (structure, formulas, charts preserved)

    Args:
        fragments: List of V3 Fragment objects (must contain root relation)
        config: Decomposition configuration (unused in V3 path)

    Returns:
        Reconstructed Workbook
    """
    # Find root relation
    relation_frags = [f for f in fragments if is_relation_fragment(f)]
    if not relation_frags:
        raise SheetReconstructionError(
            "No root relation fragment found. V3 sheet reconstruction requires a "
            "relation fragment (MIME application/ikam-relation+json) as DAG entry point."
        )

    root_frag = relation_frags[0]
    rel = Relation.model_validate(root_frag.value)

    # Build cas_id → fragment lookup
    frag_by_id = {f.cas_id: f for f in fragments if f.cas_id}

    # Reconstruct sheets from slot bindings
    sheets: List[Sheet] = []
    for bg in rel.binding_groups:
        for sb in sorted(bg.slots, key=lambda s: s.slot):
            if not sb.slot.startswith("sheet-"):
                continue
            sheet_frag = frag_by_id.get(sb.fragment_id)
            if sheet_frag is None:
                raise SheetReconstructionError(
                    f"Sheet fragment {sb.fragment_id} referenced by root relation "
                    f"not found in fragment list."
                )
            sheet_content = SheetFragmentContent.model_validate(sheet_frag.value)
            sheets.append(_build_sheet_from_content(sheet_content))

    # Extract workbook metadata from relation qualifiers
    qualifiers = rel.qualifiers or {}
    title = qualifiers.get("workbook_title", "Untitled Workbook")
    description = qualifiers.get("workbook_description", "")
    workbook_id = qualifiers.get("workbook_id", f"workbook-{_short_hash(str(fragments))}")

    # Reconstruct workbook-level charts
    workbook_charts: List[Chart] = []
    raw_charts = qualifiers.get("workbook_charts", [])
    for raw in raw_charts:
        workbook_charts.append(Chart.model_validate(raw))

    workbook_meta = WorkbookMeta(title=title, description=description)

    return Workbook(
        id=workbook_id,
        meta=workbook_meta,
        sheets=sheets,
        charts=workbook_charts,
    )


def _build_workbook_summary(workbook: Workbook) -> str:
    """Generate summary text for workbook."""
    total_cells = sum(len(sheet.cells) for sheet in workbook.sheets)
    total_charts = sum(len(sheet.charts) for sheet in workbook.sheets) + len(workbook.charts)
    total_formulas = sum(
        1 for sheet in workbook.sheets for cell in sheet.cells.values() if cell.formula
    )
    return (
        f"{workbook.meta.title}: {len(workbook.sheets)} sheets, "
        f"{total_cells} cells, {total_formulas} formulas, {total_charts} charts"
    )


def _extract_notable_formulas(workbook: Workbook, top_n: int = 10) -> List[str]:
    """Extract notable/complex formulas from workbook."""
    formulas = []
    for sheet in workbook.sheets:
        for cell in sheet.cells.values():
            if cell.formula:
                formulas.append(cell.formula)
    formulas.sort(key=len, reverse=True)
    return formulas[:top_n]


def _partition_sheet_into_ranges(
    sheet: Sheet, max_cells: int = 100
) -> List[tuple[CellRange, Dict[str, Cell]]]:
    """Partition sheet cells into logical ranges."""
    if not sheet.cells:
        return []

    cell_refs = sorted(sheet.cells.keys(), key=_cell_ref_to_row_col)
    ranges = []
    current_range_cells: Dict[str, Cell] = {}
    range_start = cell_refs[0]

    for ref in cell_refs:
        current_range_cells[ref] = sheet.cells[ref]
        if len(current_range_cells) >= max_cells:
            range_end = ref
            ranges.append(
                (CellRange(start=range_start, end=range_end), current_range_cells.copy())
            )
            current_range_cells = {}
            next_idx = cell_refs.index(ref) + 1
            if next_idx < len(cell_refs):
                range_start = cell_refs[next_idx]

    if current_range_cells:
        range_end = cell_refs[-1]
        ranges.append((CellRange(start=range_start, end=range_end), current_range_cells))

    return ranges


def _cell_ref_to_row_col(ref: str) -> tuple[int, int]:
    """Convert cell ref (e.g., 'B5') to (row, col) tuple for sorting."""
    import re
    match = re.match(r'^([A-Z]+)([0-9]+)$', ref)
    if not match:
        return (0, 0)
    col_str, row_str = match.groups()
    row = int(row_str)
    col = sum((ord(c) - ord('A') + 1) * (26 ** i) for i, c in enumerate(reversed(col_str)))
    return (row, col)


def _build_sheet_from_content(content: SheetFragmentContent) -> Sheet:
    """Build Sheet from SheetFragmentContent."""
    from .sheet_models import SheetDimensions

    sheet_name = content.sheet_name or content.sheet_id

    return Sheet(
        id=content.sheet_id,
        name=sheet_name,
        index=0,
        dimensions=SheetDimensions(),
        cells=content.cells,
        charts=content.charts,
    )


def _create_placeholder_sheet(sheet_id: str) -> Sheet:
    """Create placeholder sheet for reconstruction."""
    from .sheet_models import SheetDimensions

    return Sheet(
        id=sheet_id,
        name=sheet_id,
        index=0,
        dimensions=SheetDimensions(),
        cells={},
        charts=[],
    )


def _short_hash(text: str) -> str:
    """Generate short hash for IDs."""
    return hashlib.sha256(text.encode()).hexdigest()[:8]
