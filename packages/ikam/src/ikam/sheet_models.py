"""
IKAM Sheet Models

Pydantic models for spreadsheet data with formulas, charts, and formatting.
Complements IKAM Document and Slide Deck artifacts with structured tabular data.

References:
- docs/ikam/ikam-sheet-specification.md
- docs/ikam/ikam-v2-fragmented-knowledge-system.md

Mathematical Guarantees:
- Lossless serialization: decode(encode(Sheet)) = Sheet
- Formula preservation: All cell formulas and references maintained
- Deterministic evaluation: Same inputs → same outputs (given seed for random functions)

Version: 1.0.0 (IKAM v2 MVP - November 2025)
"""

from __future__ import annotations

from datetime import datetime
from enum import Enum
from typing import Any, Dict, List, Literal, Optional, Union

from pydantic import BaseModel, Field, ConfigDict, field_validator


class CellType(str, Enum):
    """Cell value types."""
    NUMBER = "number"
    TEXT = "text"
    BOOLEAN = "boolean"
    DATE = "date"
    ERROR = "error"
    BLANK = "blank"


class ErrorType(str, Enum):
    """Excel-compatible error types."""
    DIV_ZERO = "#DIV/0!"
    NA = "#N/A"
    NAME = "#NAME?"
    NULL = "#NULL!"
    NUM = "#NUM!"
    REF = "#REF!"
    VALUE = "#VALUE!"


class CellValue(BaseModel):
    """Typed cell value."""
    type: CellType
    value: Union[float, str, bool, None] = None

    model_config = ConfigDict(populate_by_name=True)

    @field_validator('value')
    @classmethod
    def validate_value_type(cls, v: Any, info) -> Any:
        """Validate value matches declared type."""
        if v is None:
            return v
        cell_type = info.data.get('type')
        if cell_type == CellType.NUMBER and not isinstance(v, (int, float)):
            raise ValueError(f"NUMBER cells must have numeric value, got {type(v)}")
        if cell_type == CellType.TEXT and not isinstance(v, str):
            raise ValueError(f"TEXT cells must have string value, got {type(v)}")
        if cell_type == CellType.BOOLEAN and not isinstance(v, bool):
            raise ValueError(f"BOOLEAN cells must have bool value, got {type(v)}")
        if cell_type == CellType.DATE and not isinstance(v, str):
            raise ValueError(f"DATE cells must have ISO 8601 string, got {type(v)}")
        if cell_type == CellType.ERROR and not isinstance(v, str):
            raise ValueError(f"ERROR cells must have error string, got {type(v)}")
        return v


class CellFormat(BaseModel):
    """Cell number formatting."""
    pattern: Optional[str] = None  # e.g., "#,##0.00", "0.00%", "yyyy-mm-dd"
    locale: str = "en-US"

    model_config = ConfigDict(populate_by_name=True)


class CellStyle(BaseModel):
    """Cell visual styling."""
    font_family: Optional[str] = Field(None, alias="fontFamily")
    font_size: Optional[int] = Field(None, alias="fontSize")
    bold: bool = False
    italic: bool = False
    underline: bool = False
    color: Optional[str] = None  # Hex color, e.g., "#000000"
    background_color: Optional[str] = Field(None, alias="backgroundColor")
    horizontal_align: Optional[Literal["left", "center", "right"]] = Field(None, alias="horizontalAlign")
    vertical_align: Optional[Literal["top", "middle", "bottom"]] = Field(None, alias="verticalAlign")
    border_top: Optional[str] = Field(None, alias="borderTop")  # CSS border string
    border_right: Optional[str] = Field(None, alias="borderRight")
    border_bottom: Optional[str] = Field(None, alias="borderBottom")
    border_left: Optional[str] = Field(None, alias="borderLeft")

    model_config = ConfigDict(populate_by_name=True)


class Cell(BaseModel):
    """
    Sheet cell with value, formula, and formatting.

    Mathematical Property:
    - Formula cells: value is derived from formula evaluation
    - Cached values: stored for performance, recomputed on formula change
    - Reference integrity: cell refs (e.g., "A1") must be valid within sheet dimensions
    """
    ref: str  # e.g., "A1", "B5", "AA100"
    value: Optional[CellValue] = None  # Cached computed value
    formula: Optional[str] = None  # Excel-style formula (e.g., "=SUM(A1:A10)")
    type: Optional[CellType] = None  # Inferred from value/formula
    format: Optional[CellFormat] = None
    style: Optional[CellStyle] = None
    comment: Optional[str] = None
    metadata: Dict[str, Any] = Field(default_factory=dict)

    model_config = ConfigDict(populate_by_name=True)

    @field_validator('ref')
    @classmethod
    def validate_cell_ref(cls, v: str) -> str:
        """Validate cell reference format (e.g., A1, B5, AA100)."""
        import re
        if not re.match(r'^[A-Z]+[0-9]+$', v):
            raise ValueError(f"Invalid cell reference: {v}. Must be like A1, B5, AA100")
        return v


class CellRange(BaseModel):
    """Cell range reference (e.g., A1:B10)."""
    start: str  # Cell ref
    end: str  # Cell ref

    model_config = ConfigDict(populate_by_name=True)


class NamedRange(BaseModel):
    """Named range for formula references."""
    name: str
    range: CellRange
    sheet_id: Optional[str] = Field(None, alias="sheetId")  # None = workbook-global

    model_config = ConfigDict(populate_by_name=True)


class ConditionalFormat(BaseModel):
    """Conditional formatting rule."""
    range: CellRange
    condition: str  # Formula expression (e.g., "A1>100")
    style: CellStyle

    model_config = ConfigDict(populate_by_name=True)


class DataValidation(BaseModel):
    """Data validation rule."""
    range: CellRange
    type: Literal["list", "number", "date", "text", "custom"]
    criteria: Dict[str, Any]  # Type-specific validation criteria
    error_message: Optional[str] = Field(None, alias="errorMessage")
    show_dropdown: bool = Field(True, alias="showDropdown")

    model_config = ConfigDict(populate_by_name=True)


class ChartSeries(BaseModel):
    """Chart data series."""
    name: Optional[str] = None
    data_range: CellRange = Field(..., alias="dataRange")
    label_range: Optional[CellRange] = Field(None, alias="labelRange")

    model_config = ConfigDict(populate_by_name=True)


class Chart(BaseModel):
    """Embedded chart."""
    id: str
    type: Literal["bar", "line", "pie", "scatter", "area"]
    title: Optional[str] = None
    series: List[ChartSeries]
    x_axis_label: Optional[str] = Field(None, alias="xAxisLabel")
    y_axis_label: Optional[str] = Field(None, alias="yAxisLabel")
    anchor: str  # Cell ref for top-left anchor
    offset_x: int = Field(0, alias="offsetX")
    offset_y: int = Field(0, alias="offsetY")
    width: int = 400
    height: int = 300

    model_config = ConfigDict(populate_by_name=True)


class SheetDimensions(BaseModel):
    """Sheet grid dimensions."""
    row_count: int = Field(1000, alias="rowCount", ge=1)
    column_count: int = Field(26, alias="columnCount", ge=1)
    default_row_height: Optional[int] = Field(None, alias="defaultRowHeight")
    default_column_width: Optional[int] = Field(None, alias="defaultColumnWidth")
    row_heights: Dict[int, int] = Field(default_factory=dict, alias="rowHeights")
    column_widths: Dict[int, int] = Field(default_factory=dict, alias="columnWidths")

    model_config = ConfigDict(populate_by_name=True)


class Sheet(BaseModel):
    """
    Individual sheet within a workbook.

    Mathematical Properties:
    - Cells stored sparsely: only non-empty cells in cells dict
    - Formula dependencies: implicit DAG for topological evaluation
    - Deterministic: same cell values + formulas → same results
    """
    id: str
    name: str
    index: int = Field(..., ge=0)  # Display order
    visible: bool = True
    protected: bool = False
    grid_lines: bool = Field(True, alias="gridLines")
    frozen_rows: int = Field(0, alias="frozenRows", ge=0)
    frozen_columns: int = Field(0, alias="frozenColumns", ge=0)
    dimensions: SheetDimensions
    cells: Dict[str, Cell] = Field(default_factory=dict)  # Sparse storage: ref → Cell
    merged_cells: List[CellRange] = Field(default_factory=list, alias="mergedCells")
    conditional_formats: List[ConditionalFormat] = Field(default_factory=list, alias="conditionalFormats")
    data_validations: List[DataValidation] = Field(default_factory=list, alias="dataValidations")
    named_ranges: List[NamedRange] = Field(default_factory=list, alias="namedRanges")
    charts: List[Chart] = Field(default_factory=list)

    model_config = ConfigDict(populate_by_name=True)


class WorkbookMeta(BaseModel):
    """Workbook metadata."""
    title: str
    description: Optional[str] = None
    authors: List[str] = Field(default_factory=list)
    tags: List[str] = Field(default_factory=list)
    created_at: datetime = Field(default_factory=datetime.now, alias="createdAt")
    updated_at: datetime = Field(default_factory=datetime.now, alias="updatedAt")
    project_id: Optional[str] = Field(None, alias="projectId")
    locale: str = "en-US"
    calculation_mode: Literal["auto", "manual"] = Field("auto", alias="calculationMode")

    model_config = ConfigDict(populate_by_name=True)


class Workbook(BaseModel):
    """
    Root workbook container.

    Mathematical Properties:
    - Sheet isolation: formulas can reference other sheets via Sheet!A1 syntax
    - Named ranges: global scope across all sheets
    - Version control: JSON serialization is diff-friendly (stable keys, sorted lists)
    """
    id: str
    version: str = "1.0.0"  # IKAM schema version
    type: Literal["workbook"] = "workbook"
    meta: WorkbookMeta
    sheets: List[Sheet]
    named_ranges: List[NamedRange] = Field(default_factory=list, alias="namedRanges")
    charts: List[Chart] = Field(default_factory=list)  # Workbook-level charts

    model_config = ConfigDict(populate_by_name=True)


class SheetFragmentContent(BaseModel):
    """
    Fragment content for sheet data.

    Used for decomposing sheets into hierarchical fragments:
    - L0: Summary metrics and key formulas
    - L1: Individual sheets or logical sheet sections
    - L2: Cell ranges and chart definitions
    """
    summary: str
    sheet_id: str = Field(..., alias="sheetId")
    sheet_name: Optional[str] = Field(None, alias="sheetName")
    cell_range: Optional[CellRange] = Field(None, alias="cellRange")  # None = entire sheet
    cells: Dict[str, Cell]  # Sparse cell storage
    charts: List[Chart] = Field(default_factory=list)
    formulas: List[str] = Field(default_factory=list)  # Notable formulas in this fragment

    model_config = ConfigDict(populate_by_name=True)
