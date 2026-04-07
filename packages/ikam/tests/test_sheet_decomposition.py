"""
IKAM Sheet Models and Decomposition Tests (V3 Fragment Algebra)

Validates sheet models, decomposition, and reconstruction with lossless guarantees
using V3 Fragment (cas_id, value, mime_type) and relation DAGs.

Mathematical Properties Tested:
1. Model validation: Pydantic schemas enforce type safety
2. Cell reference validation: A1, B5, AA100 format required
3. Decomposition/reconstruction losslessness: reconstruct(decompose(W)) preserves structure
4. Formula preservation: All formulas maintained across round-trip
5. Chart integrity: Chart definitions and data ranges intact

Success Criteria:
- All tests pass with 100% success rate
- Round-trip preserves all cells, formulas, charts
- No data loss in decomposition/reconstruction

Version: 2.0.0 (V3 Fragment Algebra - February 2026)
"""

import pytest
from datetime import datetime

from ikam.sheet_models import (
    Cell,
    CellRange,
    CellType,
    CellValue,
    Chart,
    ChartSeries,
    ErrorType,
    Sheet,
    SheetDimensions,
    SheetFragmentContent,
    Workbook,
    WorkbookMeta,
)
from ikam.sheet_decomposition import (
    decompose_workbook,
    reconstruct_workbook,
    SHEET_FRAGMENT_MIME,
)
from ikam.fragments import Fragment, is_relation_fragment, RELATION_MIME
from ikam.config import DecompositionConfig


@pytest.fixture
def sample_workbook() -> Workbook:
    """Create a sample workbook for testing."""
    sheet = Sheet(
        id="sheet-1",
        name="Revenue Model",
        index=0,
        dimensions=SheetDimensions(row_count=100, column_count=10),
        cells={
            "A1": Cell(ref="A1", value=CellValue(type=CellType.TEXT, value="Month")),
            "B1": Cell(ref="B1", value=CellValue(type=CellType.TEXT, value="Revenue")),
            "A2": Cell(ref="A2", value=CellValue(type=CellType.TEXT, value="Jan")),
            "B2": Cell(ref="B2", value=CellValue(type=CellType.NUMBER, value=10000), formula="=C2*D2"),
            "C2": Cell(ref="C2", value=CellValue(type=CellType.NUMBER, value=100)),
            "D2": Cell(ref="D2", value=CellValue(type=CellType.NUMBER, value=100)),
            "B3": Cell(ref="B3", value=CellValue(type=CellType.NUMBER, value=12000), formula="=SUM(B2:B2)*1.2"),
        },
        charts=[
            Chart(
                id="chart-1",
                type="line",
                title="Revenue Trend",
                series=[
                    ChartSeries(
                        name="Revenue",
                        data_range=CellRange(start="B2", end="B3"),
                        label_range=CellRange(start="A2", end="A3"),
                    )
                ],
                anchor="D5",
                width=400,
                height=300,
            )
        ],
    )

    workbook = Workbook(
        id="wb-test-001",
        meta=WorkbookMeta(
            title="Test Workbook",
            description="Sample financial model",
            authors=["Alice"],
            tags=["revenue", "model"],
        ),
        sheets=[sheet],
    )

    return workbook


def test_cell_model_validation():
    """Test Cell model validation."""
    # Valid cell
    cell = Cell(
        ref="A1",
        value=CellValue(type=CellType.NUMBER, value=42),
        formula="=B1+C1",
    )
    assert cell.ref == "A1"
    assert cell.value.type == CellType.NUMBER
    assert cell.value.value == 42
    assert cell.formula == "=B1+C1"

    # Invalid cell reference
    with pytest.raises(ValueError, match="Invalid cell reference"):
        Cell(ref="1A", value=CellValue(type=CellType.TEXT, value="Bad"))


def test_cell_value_type_validation():
    """Test CellValue type validation."""
    # Valid number
    val = CellValue(type=CellType.NUMBER, value=3.14)
    assert val.value == 3.14

    # Type mismatch
    with pytest.raises(ValueError, match="NUMBER cells must have numeric value"):
        CellValue(type=CellType.NUMBER, value="not a number")

    # Valid error
    err = CellValue(type=CellType.ERROR, value=ErrorType.DIV_ZERO.value)
    assert err.value == "#DIV/0!"


def test_workbook_serialization(sample_workbook: Workbook):
    """Test workbook JSON serialization."""
    # Serialize
    json_data = sample_workbook.model_dump(by_alias=True, mode='json')
    assert json_data["id"] == "wb-test-001"
    assert json_data["meta"]["title"] == "Test Workbook"
    assert len(json_data["sheets"]) == 1
    assert "A1" in json_data["sheets"][0]["cells"]

    # Deserialize
    reconstructed = Workbook.model_validate(json_data)
    assert reconstructed.id == sample_workbook.id
    assert reconstructed.meta.title == sample_workbook.meta.title
    assert len(reconstructed.sheets) == 1
    assert "A1" in reconstructed.sheets[0].cells


def test_decompose_workbook_creates_fragments(sample_workbook: Workbook):
    """Test workbook decomposition creates expected V3 fragments + root relation."""
    fragments = decompose_workbook(sample_workbook, artifact_id="artifact-001")

    # V3: one sheet fragment + one root relation
    sheet_frags = [f for f in fragments if f.mime_type == SHEET_FRAGMENT_MIME]
    relation_frags = [f for f in fragments if is_relation_fragment(f)]

    assert len(sheet_frags) == 1  # one per sheet
    assert len(relation_frags) == 1  # root relation

    # Every fragment has a cas_id (content-addressed)
    for f in fragments:
        assert f.cas_id is not None

    # Sheet fragment contains SheetFragmentContent (as dict in .value)
    sheet_content = SheetFragmentContent.model_validate(sheet_frags[0].value)
    assert sheet_content.sheet_id == "sheet-1"
    assert len(sheet_content.cells) == 7  # All cells in sheet


def test_reconstruct_workbook_lossless(sample_workbook: Workbook):
    """Test workbook reconstruction is lossless."""
    # Decompose
    fragments = decompose_workbook(sample_workbook, artifact_id="artifact-001")

    # Reconstruct
    reconstructed = reconstruct_workbook(fragments)

    # Validate structure
    assert len(reconstructed.sheets) == len(sample_workbook.sheets)
    reconstructed_sheet = reconstructed.sheets[0]
    original_sheet = sample_workbook.sheets[0]

    # Validate cells
    assert len(reconstructed_sheet.cells) == len(original_sheet.cells)
    for ref in original_sheet.cells:
        assert ref in reconstructed_sheet.cells
        original_cell = original_sheet.cells[ref]
        reconstructed_cell = reconstructed_sheet.cells[ref]
        assert reconstructed_cell.ref == original_cell.ref
        assert reconstructed_cell.formula == original_cell.formula
        if original_cell.value:
            assert reconstructed_cell.value.type == original_cell.value.type
            assert reconstructed_cell.value.value == original_cell.value.value

    # Validate charts
    assert len(reconstructed_sheet.charts) == len(original_sheet.charts)


def test_formula_preservation_across_round_trip(sample_workbook: Workbook):
    """Test formulas are preserved in round-trip."""
    fragments = decompose_workbook(sample_workbook, artifact_id="artifact-001")
    reconstructed = reconstruct_workbook(fragments)

    original_formulas = {
        ref: cell.formula
        for sheet in sample_workbook.sheets
        for ref, cell in sheet.cells.items()
        if cell.formula
    }

    reconstructed_formulas = {
        ref: cell.formula
        for sheet in reconstructed.sheets
        for ref, cell in sheet.cells.items()
        if cell.formula
    }

    assert reconstructed_formulas == original_formulas


def test_chart_preservation_in_fragments(sample_workbook: Workbook):
    """Test charts are preserved in V3 sheet fragments."""
    fragments = decompose_workbook(sample_workbook, artifact_id="artifact-001")

    # Find sheet fragment (by MIME type, not by level)
    sheet_frags = [f for f in fragments if f.mime_type == SHEET_FRAGMENT_MIME]
    assert len(sheet_frags) == 1

    # Deserialize SheetFragmentContent from .value dict
    sheet_content = SheetFragmentContent.model_validate(sheet_frags[0].value)
    assert len(sheet_content.charts) == 1
    assert sheet_content.charts[0].id == "chart-1"
    assert sheet_content.charts[0].type == "line"


def test_empty_workbook_decomposition():
    """Test decomposing empty workbook produces sheet fragment + root relation."""
    workbook = Workbook(
        id="wb-empty",
        meta=WorkbookMeta(title="Empty Workbook"),
        sheets=[
            Sheet(
                id="sheet-empty",
                name="Empty Sheet",
                index=0,
                dimensions=SheetDimensions(),
                cells={},
            )
        ],
    )

    fragments = decompose_workbook(workbook, artifact_id="artifact-empty")

    # V3: 1 sheet fragment + 1 root relation = 2
    assert len(fragments) == 2
    sheet_frags = [f for f in fragments if f.mime_type == SHEET_FRAGMENT_MIME]
    relation_frags = [f for f in fragments if is_relation_fragment(f)]
    assert len(sheet_frags) == 1
    assert len(relation_frags) == 1


def test_multi_sheet_workbook():
    """Test workbook with multiple sheets produces one fragment per sheet."""
    workbook = Workbook(
        id="wb-multi",
        meta=WorkbookMeta(title="Multi-Sheet Workbook"),
        sheets=[
            Sheet(
                id="sheet-1",
                name="Data",
                index=0,
                dimensions=SheetDimensions(),
                cells={"A1": Cell(ref="A1", value=CellValue(type=CellType.NUMBER, value=1))},
            ),
            Sheet(
                id="sheet-2",
                name="Analysis",
                index=1,
                dimensions=SheetDimensions(),
                cells={"B2": Cell(ref="B2", value=CellValue(type=CellType.NUMBER, value=2))},
            ),
        ],
    )

    fragments = decompose_workbook(workbook, artifact_id="artifact-multi")

    # V3: 2 sheet fragments + 1 root relation
    sheet_frags = [f for f in fragments if f.mime_type == SHEET_FRAGMENT_MIME]
    assert len(sheet_frags) == 2  # One per sheet


def test_large_sheet_decomposition():
    """Test large sheets produce one fragment per sheet in V3 (no sub-range partitioning)."""
    # Create sheet with many cells
    cells = {
        f"A{i}": Cell(ref=f"A{i}", value=CellValue(type=CellType.NUMBER, value=i))
        for i in range(1, 201)  # 200 cells
    }

    workbook = Workbook(
        id="wb-large",
        meta=WorkbookMeta(title="Large Workbook"),
        sheets=[
            Sheet(
                id="sheet-large",
                name="Large Sheet",
                index=0,
                dimensions=SheetDimensions(row_count=500),
                cells=cells,
            )
        ],
    )

    fragments = decompose_workbook(workbook, artifact_id="artifact-large")

    # V3: 1 sheet fragment + 1 root relation (no sub-range partitioning)
    sheet_frags = [f for f in fragments if f.mime_type == SHEET_FRAGMENT_MIME]
    assert len(sheet_frags) == 1

    # Verify all 200 cells are in the sheet fragment
    sheet_content = SheetFragmentContent.model_validate(sheet_frags[0].value)
    assert len(sheet_content.cells) == 200
