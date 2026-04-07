"""Tabular Profile — Schema + Rows for specialized structured data.

This profile specializes StructuredDataIR for tabular data with explicit 
axes and metadata.
"""

from __future__ import annotations

from typing import Any, Dict, List, Optional

from pydantic import BaseModel, Field

from ikam.ir import StructuredDataIR, Axis, AxisRole
from modelado.profiles import TABULAR_V1


class ColumnDef(BaseModel):
    """Column definition for a TabularIR."""
    name: str
    dtype: str  # "text", "numeric", "date", "boolean"
    unit: Optional[str] = None
    format_spec: Optional[str] = None  # e.g., "currency:usd:2dp"


class TableData(BaseModel):
    """A table expressed as schema (columns) + rows."""
    columns: List[ColumnDef] = Field(default_factory=list)
    rows: List[Dict[str, Any]] = Field(default_factory=list)


def create_tabular_ir(artifact_id: str, columns: List[ColumnDef], rows: List[Dict[str, Any]]) -> StructuredDataIR:
    """Create a StructuredDataIR instance with the Tabular profile."""
    table = TableData(columns=columns, rows=rows)
    axes = [
        Axis(name="rows", role=AxisRole.INDEX),
        Axis(name="columns", role=AxisRole.COLUMNS, labels=[c.name for c in columns])
    ]
    return StructuredDataIR(
        artifact_id=artifact_id,
        profile=TABULAR_V1,
        axes=axes,
        data=table.model_dump(mode="json"),
    )
