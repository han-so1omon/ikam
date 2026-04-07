# IKAM Sheet Specification

_Version 1.0.0 | October 2025_

## Overview

The **IKAM Sheet** is Narraciones' canonical format for structured tabular data with formulas, formatting, charts, and analytics. It complements IKAM Document (semantic content) and IKAM Slide Deck (presentations) by providing spreadsheet capabilities for financial modeling, data analysis, and scenario planning.

IKAM Sheet enables:
- **Formula-driven models** — Excel-like formulas with cell references
- **Multi-sheet workbooks** — organize related data across sheets
- **Rich formatting** — cell styles, conditional formatting, data validation
- **Embedded analytics** — charts, pivot tables, calculated metrics
- **Version control** — diff-friendly JSON representation
- **Export flexibility** — Excel (XLSX), Google Sheets, CSV, HTML tables

---

## Design Principles

1. **Formula-First** — Cells store formulas, not just values; recalculation on demand
2. **Structured & Typed** — Strong typing for cell values (number, text, date, boolean, error)
3. **Diffable** — JSON-based with stable cell references; Git-friendly
4. **Composable** — Sheets reference each other; charts reference data ranges
5. **Standard-Aligned** — Use Excel formula syntax where applicable
6. **Export-Agnostic** — One canonical source → XLSX, Google Sheets, CSV, HTML
7. **Integration-Ready** — Cells can reference IKAM Project artifacts (models, documents)

---

## Root Workbook Schema

```typescript
interface Workbook {
  id: string;                    // UUID
  version: string;               // IKAM schema version (semver, e.g., "1.0.0")
  type: "workbook";
  meta: WorkbookMeta;
  sheets: Sheet[];
  namedRanges?: NamedRange[];    // Global named ranges
  charts?: Chart[];              // Charts defined at workbook level
}

interface WorkbookMeta {
  title: string;
  description?: string;
  authors?: string[];
  tags?: string[];
  createdAt: string;             // ISO 8601
  updatedAt: string;
  projectId?: string;            // Reference to IKAM Project
  locale?: string;               // e.g., "en-US" for number formatting
  calculationMode?: "auto" | "manual";
}
```

---

## Sheet Schema

```typescript
interface Sheet {
  id: string;                    // UUID
  name: string;                  // Display name (must be unique within workbook)
  index: number;                 // Display order (0-based)
  visible: boolean;
  protected?: boolean;
  gridLines?: boolean;
  frozenRows?: number;           // Number of rows frozen at top
  frozenColumns?: number;        // Number of columns frozen at left
  dimensions: SheetDimensions;
  cells: Record<CellRef, Cell>;  // Sparse cell storage
  mergedCells?: CellRange[];
  conditionalFormats?: ConditionalFormat[];
  dataValidations?: DataValidation[];
  namedRanges?: NamedRange[];    // Sheet-local named ranges
  charts?: Chart[];              // Charts embedded in this sheet
}

interface SheetDimensions {
  rowCount: number;              // Total rows (default 1000)
  columnCount: number;           // Total columns (default 26, i.e., A-Z)
  defaultRowHeight?: number;     // In points
  defaultColumnWidth?: number;   // In points
  rowHeights?: Record<number, number>;      // Custom heights
  columnWidths?: Record<number, number>;    // Custom widths
}

type CellRef = string;           // e.g., "A1", "B5", "AA100"
```

---

## Cell Schema

```typescript
interface Cell {
  ref: CellRef;                  // e.g., "B5"
  value?: CellValue;             // Computed value (cached)
  formula?: string;              // Excel-style formula (e.g., "=A1+B1")
  type?: CellType;               // Inferred from value/formula
  format?: CellFormat;
  style?: CellStyle;
  comment?: string;
  metadata?: Record<string, any>;
}

type CellValue = 
  | { type: "number"; value: number }
  | { type: "text"; value: string }
  | { type: "boolean"; value: boolean }
  | { type: "date"; value: string }      // ISO 8601
  | { type: "error"; value: ErrorType }
  | { type: "blank" };

type CellType = "number" | "text" | "boolean" | "date" | "error" | "blank";

type ErrorType = 
  | "#DIV/0!"     // Division by zero
  | "#N/A"        // Value not available
  | "#NAME?"      // Unrecognized function name
  | "#NULL!"      // Invalid range intersection
  | "#NUM!"       // Invalid numeric value
  | "#REF!"       // Invalid cell reference
  | "#VALUE!";    // Wrong value type

interface CellFormat {
  numberFormat?: string;         // e.g., "0.00", "#,##0.00", "mm/dd/yyyy"
  dateFormat?: string;
  textFormat?: "plain" | "rich";
}

interface CellStyle {
  font?: FontStyle;
  fill?: FillStyle;
  border?: BorderStyle;
  alignment?: AlignmentStyle;
}

interface FontStyle {
  family?: string;               // e.g., "Arial", "Times New Roman"
  size?: number;                 // Points
  bold?: boolean;
  italic?: boolean;
  underline?: boolean;
  strikethrough?: boolean;
  color?: string;                // Hex color, e.g., "#FF0000"
}

interface FillStyle {
  type: "solid" | "gradient" | "pattern";
  color?: string;                // Primary color
  backgroundColor?: string;      // For patterns
  gradient?: GradientDef;
}

interface BorderStyle {
  top?: BorderEdge;
  right?: BorderEdge;
  bottom?: BorderEdge;
  left?: BorderEdge;
}

interface BorderEdge {
  style: "thin" | "medium" | "thick" | "dashed" | "dotted" | "double";
  color?: string;
}

interface AlignmentStyle {
  horizontal?: "left" | "center" | "right" | "justify";
  vertical?: "top" | "middle" | "bottom";
  wrapText?: boolean;
  textRotation?: number;         // Degrees (0-360)
}
```

---

## Formula System

### Formula Syntax

IKAM Sheet uses Excel-compatible formula syntax:

```
=SUM(A1:A10)
=IF(B5>100, "High", "Low")
=VLOOKUP(C2, A1:B100, 2, FALSE)
=A1 + B1 * 2
=Sheet2!A5
=NamedRange * 0.05
```

### Supported Functions

**Math & Statistics:**
- `SUM`, `AVERAGE`, `MIN`, `MAX`, `COUNT`, `COUNTA`, `COUNTIF`
- `MEDIAN`, `MODE`, `STDEV`, `VAR`, `PERCENTILE`
- `ROUND`, `ROUNDUP`, `ROUNDDOWN`, `FLOOR`, `CEILING`
- `ABS`, `SQRT`, `POWER`, `EXP`, `LN`, `LOG`

**Logical:**
- `IF`, `AND`, `OR`, `NOT`, `XOR`
- `IFERROR`, `IFNA`, `IFS`, `SWITCH`

**Lookup & Reference:**
- `VLOOKUP`, `HLOOKUP`, `INDEX`, `MATCH`, `OFFSET`
- `INDIRECT`, `ROW`, `COLUMN`, `ROWS`, `COLUMNS`

**Text:**
- `CONCATENATE`, `CONCAT`, `TEXTJOIN`, `LEFT`, `RIGHT`, `MID`
- `UPPER`, `LOWER`, `PROPER`, `TRIM`, `LEN`
- `FIND`, `SEARCH`, `REPLACE`, `SUBSTITUTE`

**Date & Time:**
- `TODAY`, `NOW`, `DATE`, `TIME`, `YEAR`, `MONTH`, `DAY`
- `HOUR`, `MINUTE`, `SECOND`, `WEEKDAY`, `EOMONTH`
- `DATEDIF`, `NETWORKDAYS`, `WORKDAY`

**Financial:**
- `NPV`, `IRR`, `XIRR`, `PMT`, `FV`, `PV`
- `RATE`, `NPER`, `CUMIPMT`, `CUMPRINC`

**Custom (Narraciones-specific):**
- `ARTIFACT(artifactId, field)` — Reference IKAM Project artifact field
- `MODEL(modelId, output)` — Reference economic/story model output
- `SCENARIO(scenarioId, metric)` — Pull scenario metric value

### Cell References

```typescript
type CellReference = 
  | SimpleCellRef      // "A1", "$B$5" (absolute)
  | RangeRef           // "A1:B10", "Sheet2!C1:D20"
  | NamedRangeRef;     // "Revenue", "CostOfGoodsSold"

interface SimpleCellRef {
  sheet?: string;      // Optional sheet name
  column: string;      // "A", "B", ..., "ZZ"
  row: number;         // 1-based
  absolute?: {
    column: boolean;   // True if $A (absolute column)
    row: boolean;      // True if $1 (absolute row)
  };
}

interface RangeRef {
  sheet?: string;
  start: SimpleCellRef;
  end: SimpleCellRef;
}
```

---

## Named Ranges

```typescript
interface NamedRange {
  name: string;                  // e.g., "Revenue", "Q1_Sales"
  scope: "workbook" | string;    // "workbook" or sheet ID
  ref: CellRange;                // Range this name refers to
  comment?: string;
}

interface CellRange {
  sheet: string;                 // Sheet ID
  start: CellRef;                // e.g., "A1"
  end: CellRef;                  // e.g., "B10"
}
```

---

## Conditional Formatting

```typescript
interface ConditionalFormat {
  id: string;
  range: CellRange;
  rules: FormatRule[];
}

interface FormatRule {
  type: RuleType;
  condition: Condition;
  format: CellStyle;
  stopIfTrue?: boolean;
}

type RuleType = 
  | "cellValue"        // Compare cell value to constant
  | "formula"          // Custom formula returns TRUE
  | "colorScale"       // Gradient based on value
  | "dataBar"          // Bar chart in cell
  | "iconSet";         // Traffic light icons

interface Condition {
  operator?: "greaterThan" | "lessThan" | "equal" | "between" | "contains";
  value?: any;
  value2?: any;        // For "between"
  formula?: string;    // For formula-based rules
}
```

---

## Data Validation

```typescript
interface DataValidation {
  id: string;
  range: CellRange;
  rule: ValidationRule;
  errorMessage?: string;
  errorTitle?: string;
  errorStyle?: "stop" | "warning" | "information";
}

interface ValidationRule {
  type: ValidationType;
  operator?: "between" | "notBetween" | "equal" | "notEqual" | "greaterThan" | "lessThan";
  formula1?: string;   // Min value or list formula
  formula2?: string;   // Max value
  allowBlank?: boolean;
  showDropdown?: boolean;
}

type ValidationType = 
  | "whole"            // Integer
  | "decimal"          // Decimal number
  | "list"             // Dropdown list
  | "date"
  | "time"
  | "textLength"
  | "custom";          // Custom formula
```

---

## Charts

```typescript
interface Chart {
  id: string;
  type: ChartType;
  title?: string;
  dataRanges: DataRange[];
  options: ChartOptions;
  position?: ChartPosition;      // For sheet-embedded charts
}

type ChartType = 
  | "line" | "bar" | "column" | "area" | "pie" | "doughnut"
  | "scatter" | "bubble" | "radar" | "combo"
  | "waterfall" | "funnel" | "treemap" | "heatmap";

interface DataRange {
  label: string;                 // Series name
  range: CellRange;
  axis?: "primary" | "secondary";
}

interface ChartOptions {
  width?: number;
  height?: number;
  legend?: LegendOptions;
  axes?: AxisOptions;
  colors?: string[];             // Color palette
  vegaLiteSpec?: object;         // Full Vega-Lite spec (optional override)
}

interface ChartPosition {
  sheet: string;
  anchor: CellRef;               // Top-left anchor cell
  offsetX?: number;              // Pixels from anchor
  offsetY?: number;
}
```

---

## Pivot Tables

```typescript
interface PivotTable {
  id: string;
  name: string;
  sourceRange: CellRange;        // Raw data range
  rows: PivotField[];            // Row dimensions
  columns: PivotField[];         // Column dimensions
  values: PivotValue[];          // Aggregated values
  filters?: PivotFilter[];
  options?: PivotOptions;
}

interface PivotField {
  field: string;                 // Column name in source
  label?: string;                // Display name
  sort?: "asc" | "desc";
}

interface PivotValue {
  field: string;
  aggregation: "sum" | "count" | "average" | "min" | "max" | "stdev";
  label?: string;
  numberFormat?: string;
}

interface PivotFilter {
  field: string;
  values: any[];                 // Selected values
}

interface PivotOptions {
  grandTotals?: boolean;
  subtotals?: boolean;
  compactLayout?: boolean;
}
```

---

## Calculation Engine

### Dependency Graph

The calculation engine builds a directed acyclic graph (DAG) of cell dependencies:

```python
from typing import Dict, Set, List
from dataclasses import dataclass

@dataclass
class CellNode:
    ref: str
    formula: str | None
    value: Any
    dependencies: Set[str]  # Cells this cell depends on
    dependents: Set[str]    # Cells that depend on this cell

class CalculationEngine:
    def __init__(self, workbook: Workbook):
        self.workbook = workbook
        self.graph: Dict[str, CellNode] = {}
        self.build_graph()
    
    def build_graph(self):
        """Build dependency graph from all cells with formulas."""
        for sheet in self.workbook.sheets:
            for cell_ref, cell in sheet.cells.items():
                if cell.formula:
                    deps = self.parse_dependencies(cell.formula)
                    self.graph[f"{sheet.id}!{cell_ref}"] = CellNode(
                        ref=cell_ref,
                        formula=cell.formula,
                        value=cell.value,
                        dependencies=deps,
                        dependents=set()
                    )
    
    def parse_dependencies(self, formula: str) -> Set[str]:
        """Extract cell references from formula."""
        # Use regex or formula parser to find A1, B5, Sheet2!C10, etc.
        pass
    
    def recalculate(self, changed_cells: List[str]) -> List[str]:
        """Topological sort from changed cells, recalculate dependents."""
        visited = set()
        order = []
        
        for cell_ref in changed_cells:
            self._topo_sort(cell_ref, visited, order)
        
        for cell_ref in order:
            self._evaluate_cell(cell_ref)
        
        return order
    
    def _topo_sort(self, ref: str, visited: Set[str], order: List[str]):
        """Depth-first search for topological ordering."""
        if ref in visited:
            return
        visited.add(ref)
        
        node = self.graph.get(ref)
        if node:
            for dep in node.dependents:
                self._topo_sort(dep, visited, order)
        
        order.append(ref)
    
    def _evaluate_cell(self, ref: str):
        """Evaluate formula and update cell value."""
        node = self.graph[ref]
        try:
            # Parse and evaluate formula
            result = self.evaluate_formula(node.formula)
            node.value = result
        except Exception as e:
            node.value = {"type": "error", "value": "#VALUE!"}
```

### Circular Reference Detection

```python
def detect_circular_references(self) -> List[List[str]]:
    """Find all circular dependency cycles."""
    cycles = []
    visited = set()
    rec_stack = []
    
    for node_ref in self.graph:
        if node_ref not in visited:
            self._detect_cycle(node_ref, visited, rec_stack, cycles)
    
    return cycles

def _detect_cycle(self, ref: str, visited: Set[str], rec_stack: List[str], cycles: List[List[str]]):
    visited.add(ref)
    rec_stack.append(ref)
    
    node = self.graph[ref]
    for dep in node.dependencies:
        if dep not in visited:
            self._detect_cycle(dep, visited, rec_stack, cycles)
        elif dep in rec_stack:
            # Found cycle
            cycle_start = rec_stack.index(dep)
            cycles.append(rec_stack[cycle_start:] + [dep])
    
    rec_stack.pop()
```

---

## Export Formats

### Excel (XLSX)

Use `openpyxl` or `xlsxwriter` to generate Excel files:

```python
from openpyxl import Workbook as XLWorkbook
from openpyxl.styles import Font, Fill, Border, Alignment

def export_to_xlsx(workbook: Workbook, output_path: str):
    wb = XLWorkbook()
    
    for sheet in workbook.sheets:
        ws = wb.create_sheet(title=sheet.name)
        
        # Set dimensions
        ws.freeze_panes = f"{chr(65 + sheet.frozenColumns or 0)}{sheet.frozenRows or 1}"
        
        # Write cells
        for cell_ref, cell in sheet.cells.items():
            xl_cell = ws[cell_ref]
            
            # Set value
            if cell.value:
                xl_cell.value = cell.value.get("value")
            
            # Set formula
            if cell.formula:
                xl_cell.value = cell.formula
            
            # Apply styles
            if cell.style:
                if cell.style.font:
                    xl_cell.font = Font(
                        name=cell.style.font.family,
                        size=cell.style.font.size,
                        bold=cell.style.font.bold,
                        italic=cell.style.font.italic,
                        color=cell.style.font.color
                    )
                # ... fill, border, alignment
        
        # Apply conditional formatting
        for cf in sheet.conditionalFormats or []:
            # Use openpyxl conditional formatting API
            pass
    
    wb.save(output_path)
```

### Google Sheets

Use Google Sheets API to create/update spreadsheets:

```python
from googleapiclient.discovery import build

def export_to_google_sheets(workbook: Workbook, credentials):
    service = build('sheets', 'v4', credentials=credentials)
    
    # Create spreadsheet
    spreadsheet = service.spreadsheets().create(body={
        "properties": {
            "title": workbook.meta.title
        },
        "sheets": [
            {
                "properties": {
                    "title": sheet.name,
                    "gridProperties": {
                        "rowCount": sheet.dimensions.rowCount,
                        "columnCount": sheet.dimensions.columnCount,
                        "frozenRowCount": sheet.frozenRows or 0,
                        "frozenColumnCount": sheet.frozenColumns or 0
                    }
                }
            }
            for sheet in workbook.sheets
        ]
    }).execute()
    
    # Batch update cells
    requests = []
    for sheet in workbook.sheets:
        for cell_ref, cell in sheet.cells.items():
            row, col = parse_cell_ref(cell_ref)
            requests.append({
                "updateCells": {
                    "range": {
                        "sheetId": sheet.index,
                        "startRowIndex": row - 1,
                        "endRowIndex": row,
                        "startColumnIndex": col - 1,
                        "endColumnIndex": col
                    },
                    "rows": [{
                        "values": [{
                            "userEnteredValue": format_value(cell.value),
                            "userEnteredFormat": format_style(cell.style)
                        }]
                    }],
                    "fields": "userEnteredValue,userEnteredFormat"
                }
            })
    
    service.spreadsheets().batchUpdate(
        spreadsheetId=spreadsheet['spreadsheetId'],
        body={"requests": requests}
    ).execute()
    
    return spreadsheet
```

### CSV

Export individual sheets as CSV:

```python
import csv

def export_sheet_to_csv(sheet: Sheet, output_path: str):
    max_row = max((parse_cell_ref(ref)[0] for ref in sheet.cells.keys()), default=0)
    max_col = max((parse_cell_ref(ref)[1] for ref in sheet.cells.keys()), default=0)
    
    with open(output_path, 'w', newline='') as f:
        writer = csv.writer(f)
        
        for row in range(1, max_row + 1):
            row_data = []
            for col in range(1, max_col + 1):
                cell_ref = format_cell_ref(row, col)
                cell = sheet.cells.get(cell_ref)
                
                if cell and cell.value:
                    row_data.append(str(cell.value.get("value", "")))
                else:
                    row_data.append("")
            
            writer.writerow(row_data)
```

### HTML Table

Render sheet as HTML table with inline styles:

```python
from jinja2 import Template

HTML_TABLE_TEMPLATE = """
<!DOCTYPE html>
<html>
<head>
    <title>{{ sheet.name }}</title>
    <style>
        table { border-collapse: collapse; width: 100%; }
        th, td { border: 1px solid #ddd; padding: 8px; text-align: left; }
        th { background-color: #f2f2f2; font-weight: bold; }
    </style>
</head>
<body>
    <h1>{{ sheet.name }}</h1>
    <table>
        {% for row in rows %}
        <tr>
            {% for cell in row %}
            <td style="{{ cell.style }}">{{ cell.display_value }}</td>
            {% endfor %}
        </tr>
        {% endfor %}
    </table>
</body>
</html>
"""

def export_sheet_to_html(sheet: Sheet) -> str:
    rows = build_rows_from_cells(sheet.cells, sheet.dimensions)
    template = Template(HTML_TABLE_TEMPLATE)
    return template.render(sheet=sheet, rows=rows)
```

---

## Storage Schema (PostgreSQL)

```sql
-- Workbooks table
CREATE TABLE workbooks (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID REFERENCES projects(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    description TEXT,
    authors TEXT[],
    tags TEXT[],
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    calculation_mode TEXT DEFAULT 'auto' CHECK (calculation_mode IN ('auto', 'manual')),
    version TEXT NOT NULL DEFAULT '1.0.0',
    locale TEXT DEFAULT 'en-US'
);

-- Sheets table
CREATE TABLE sheets (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workbook_id UUID NOT NULL REFERENCES workbooks(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    index INTEGER NOT NULL,
    visible BOOLEAN DEFAULT true,
    protected BOOLEAN DEFAULT false,
    grid_lines BOOLEAN DEFAULT true,
    frozen_rows INTEGER DEFAULT 0,
    frozen_columns INTEGER DEFAULT 0,
    row_count INTEGER DEFAULT 1000,
    column_count INTEGER DEFAULT 26,
    default_row_height NUMERIC,
    default_column_width NUMERIC,
    row_heights JSONB,        -- {"5": 30, "10": 50}
    column_widths JSONB,      -- {"2": 120, "5": 80}
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(workbook_id, name),
    UNIQUE(workbook_id, index)
);

-- Cells table (sparse storage)
CREATE TABLE cells (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    sheet_id UUID NOT NULL REFERENCES sheets(id) ON DELETE CASCADE,
    ref TEXT NOT NULL,        -- e.g., "A1", "B5"
    row INTEGER NOT NULL,
    col INTEGER NOT NULL,
    value JSONB,              -- {"type": "number", "value": 42}
    formula TEXT,             -- e.g., "=SUM(A1:A10)"
    cell_type TEXT CHECK (cell_type IN ('number', 'text', 'boolean', 'date', 'error', 'blank')),
    format JSONB,             -- {"numberFormat": "0.00"}
    style JSONB,              -- Full CellStyle object
    comment TEXT,
    metadata JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(sheet_id, ref)
);

CREATE INDEX idx_cells_sheet_id ON cells(sheet_id);
CREATE INDEX idx_cells_ref ON cells(sheet_id, ref);
CREATE INDEX idx_cells_row_col ON cells(sheet_id, row, col);

-- Named ranges
CREATE TABLE named_ranges (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workbook_id UUID REFERENCES workbooks(id) ON DELETE CASCADE,
    sheet_id UUID REFERENCES sheets(id) ON DELETE CASCADE,  -- NULL for workbook-scoped
    name TEXT NOT NULL,
    range_ref JSONB NOT NULL,  -- {"sheet": "sheet-id", "start": "A1", "end": "B10"}
    comment TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(workbook_id, name),
    CHECK (workbook_id IS NOT NULL OR sheet_id IS NOT NULL)
);

-- Charts
CREATE TABLE charts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workbook_id UUID REFERENCES workbooks(id) ON DELETE CASCADE,
    sheet_id UUID REFERENCES sheets(id) ON DELETE SET NULL,  -- NULL for workbook-level charts
    type TEXT NOT NULL,
    title TEXT,
    data_ranges JSONB NOT NULL,
    options JSONB,
    position JSONB,           -- For sheet-embedded charts
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Conditional formats
CREATE TABLE conditional_formats (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    sheet_id UUID NOT NULL REFERENCES sheets(id) ON DELETE CASCADE,
    range_ref JSONB NOT NULL,
    rules JSONB NOT NULL,     -- Array of FormatRule objects
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Data validations
CREATE TABLE data_validations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    sheet_id UUID NOT NULL REFERENCES sheets(id) ON DELETE CASCADE,
    range_ref JSONB NOT NULL,
    rule JSONB NOT NULL,
    error_message TEXT,
    error_title TEXT,
    error_style TEXT CHECK (error_style IN ('stop', 'warning', 'information')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

---

## Validation (Pydantic)

```python
from pydantic import BaseModel, Field, field_validator
from typing import Literal, Optional, Dict, List, Any
from datetime import datetime

class WorkbookMeta(BaseModel):
    title: str
    description: Optional[str] = None
    authors: Optional[List[str]] = None
    tags: Optional[List[str]] = None
    created_at: datetime
    updated_at: datetime
    project_id: Optional[str] = None
    locale: str = "en-US"
    calculation_mode: Literal["auto", "manual"] = "auto"

class CellValue(BaseModel):
    type: Literal["number", "text", "boolean", "date", "error", "blank"]
    value: Any

class Cell(BaseModel):
    ref: str  # e.g., "A1"
    value: Optional[CellValue] = None
    formula: Optional[str] = None
    type: Optional[Literal["number", "text", "boolean", "date", "error", "blank"]] = None
    format: Optional[Dict[str, Any]] = None
    style: Optional[Dict[str, Any]] = None
    comment: Optional[str] = None
    metadata: Optional[Dict[str, Any]] = None
    
    @field_validator('ref')
    @classmethod
    def validate_ref(cls, v: str) -> str:
        # Validate cell reference format (A1, B5, AA100, etc.)
        import re
        if not re.match(r'^[A-Z]+\d+$', v):
            raise ValueError(f"Invalid cell reference: {v}")
        return v

class Sheet(BaseModel):
    id: str
    name: str
    index: int
    visible: bool = True
    protected: bool = False
    grid_lines: bool = True
    frozen_rows: int = 0
    frozen_columns: int = 0
    dimensions: Dict[str, Any]
    cells: Dict[str, Cell]
    merged_cells: Optional[List[Dict[str, Any]]] = None
    conditional_formats: Optional[List[Dict[str, Any]]] = None
    data_validations: Optional[List[Dict[str, Any]]] = None
    named_ranges: Optional[List[Dict[str, Any]]] = None
    charts: Optional[List[Dict[str, Any]]] = None

class Workbook(BaseModel):
    id: str
    version: str = "1.0.0"
    type: Literal["workbook"] = "workbook"
    meta: WorkbookMeta
    sheets: List[Sheet]
    named_ranges: Optional[List[Dict[str, Any]]] = None
    charts: Optional[List[Dict[str, Any]]] = None
    
    @field_validator('sheets')
    @classmethod
    def validate_unique_sheet_names(cls, v: List[Sheet]) -> List[Sheet]:
        names = [sheet.name for sheet in v]
        if len(names) != len(set(names)):
            raise ValueError("Sheet names must be unique within a workbook")
        return v
```

---

## Integration with IKAM Project

### Artifact Registration

IKAM Sheets register as `WorkbookArtifact` in the IKAM Project artifact registry:

```typescript
interface WorkbookArtifact {
  type: "workbook";
  id: string;
  workbookId: string;       // Reference to workbooks table
  meta: ArtifactMeta;
}
```

### Cross-Referencing

Workbook cells can reference other IKAM artifacts:

```
=ARTIFACT("doc-123", "metrics.revenue")
=MODEL("econ-456", "npv")
=SCENARIO("scenario-789", "opex")
```

These custom functions resolve by:
1. Querying IKAM Project artifact registry
2. Extracting requested field/output
3. Caching value with dependency tracking

### Derivation Tracking

Charts and pivot tables derived from sheet data register as derivations:

```typescript
{
  source: {
    type: "workbook",
    id: "workbook-123",
    range: "Sheet1!A1:B100"
  },
  transform: "pivot_table",
  derived: {
    type: "chart",
    id: "chart-456"
  }
}
```

---

## Migration Path

### Phase 1: Foundation (Sprint 6)
- Define Pydantic models for workbooks, sheets, cells
- Add database migrations (workbooks, sheets, cells tables)
- Implement basic CRUD API (`/api/workbooks`, `/api/sheets`, `/api/cells`)
- Build formula parser (lexer + AST)

### Phase 2: Calculation Engine (Sprint 7)
- Implement dependency graph builder
- Add topological sort for recalculation
- Support basic functions (SUM, AVERAGE, IF, VLOOKUP)
- Add circular reference detection

### Phase 3: Formatting & Validation (Sprint 7)
- Implement cell styles (font, fill, border, alignment)
- Add conditional formatting support
- Implement data validation rules
- Add named ranges

### Phase 4: Export (Sprint 7)
- XLSX export via openpyxl
- Google Sheets export via Sheets API
- CSV export for individual sheets
- HTML table rendering

### Phase 5: Advanced Features (Post-Sprint 7)
- Charts (integrated with Vega-Lite)
- Pivot tables
- Custom functions (ARTIFACT, MODEL, SCENARIO)
- Real-time collaboration (operational transforms)

---

## Testing Strategy

### Unit Tests
- Formula parser: tokenization, AST construction, error handling
- Calculation engine: dependency graph, topological sort, circular detection
- Cell validation: reference format, value types, style schemas
- Export: XLSX structure, CSV format, HTML rendering

### Integration Tests
- End-to-end formula evaluation (complex nested formulas)
- Cross-sheet references
- Named ranges resolution
- Export round-trip (IKAM → XLSX → IKAM)

### Performance Tests
- Large workbooks (10,000+ cells)
- Complex formulas (100+ dependencies)
- Recalculation time (changed cell → all dependents)
- Export time (XLSX generation for large sheets)

---

## Appendix: Formula Grammar (EBNF)

```ebnf
formula       ::= "=" expression
expression    ::= term ( ("+" | "-") term )*
term          ::= factor ( ("*" | "/") factor )*
factor        ::= power ( "^" power )*
power         ::= unary | primary
unary         ::= ("+" | "-") power
primary       ::= number | string | boolean | cell_ref | range_ref | function_call | "(" expression ")"

number        ::= [0-9]+ ("." [0-9]+)? ([eE] [+-]? [0-9]+)?
string        ::= '"' [^"]* '"'
boolean       ::= "TRUE" | "FALSE"
cell_ref      ::= [A-Z]+ [0-9]+ | sheet_name "!" [A-Z]+ [0-9]+
range_ref     ::= cell_ref ":" cell_ref
sheet_name    ::= [A-Za-z0-9_]+

function_call ::= function_name "(" arg_list? ")"
function_name ::= [A-Z]+
arg_list      ::= expression ("," expression)*
```

---

## Success Metrics

- **Formula Coverage:** 90%+ of common Excel functions supported
- **Calculation Performance:** <100ms recalculation for 1000-cell workbook
- **Export Fidelity:** 95%+ style/format preservation in XLSX export
- **Cross-Reference Accuracy:** 100% correct resolution of ARTIFACT/MODEL references
- **Collaboration-Ready:** Operational transform conflicts <1% in multi-user editing

---

## References

- [OpenPyXL Documentation](https://openpyxl.readthedocs.io/)
- [Google Sheets API v4](https://developers.google.com/sheets/api)
- [Excel Formula Reference](https://support.microsoft.com/en-us/office/excel-functions-alphabetical-b3944572-255d-4efb-bb96-c6d90033e188)
- [IKAM Project Specification](./ikam-project-specification.md)
- [IKAM Document/Slide Specification](./ikam-specification.md)
