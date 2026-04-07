"""Tests for the Tabular Profile (Plan D Stage 2).

Covers:
  - create_tabular_ir() produces valid StructuredDataIR with tabular@1 profile
  - Round-trip: XLSX-like dict → TabularIR → render dict → matches original
  - RFC 6902 structured patch via ApplyOperator on tabular data dict
"""
from __future__ import annotations

import pytest
from modelado.profiles.tabular import ColumnDef, TableData, create_tabular_ir
from modelado.profiles import TABULAR_V1
from modelado.operators.monadic import ApplyOperator
from modelado.operators.core import OperatorEnv, OperatorParams
from modelado.environment_scope import EnvironmentScope

_DEV_SCOPE = EnvironmentScope(ref="refs/heads/run/run/test")


# -- Fixtures --

@pytest.fixture
def sample_columns():
    return [
        ColumnDef(name="quarter", dtype="text"),
        ColumnDef(name="revenue", dtype="numeric", unit="USD"),
        ColumnDef(name="growth_pct", dtype="numeric", format_spec="percent:1dp"),
    ]


@pytest.fixture
def sample_rows():
    return [
        {"quarter": "Q1", "revenue": 1_200_000, "growth_pct": 0.25},
        {"quarter": "Q2", "revenue": 1_500_000, "growth_pct": 0.28},
    ]


@pytest.fixture
def tabular_ir(sample_columns, sample_rows):
    return create_tabular_ir("artifact-001", sample_columns, sample_rows)


# -- Structure tests --

def test_create_tabular_ir_profile(tabular_ir):
    assert tabular_ir.profile == TABULAR_V1


def test_create_tabular_ir_artifact_id(tabular_ir):
    assert tabular_ir.artifact_id == "artifact-001"


def test_create_tabular_ir_axes(tabular_ir):
    names = {ax.name for ax in tabular_ir.axes}
    assert "rows" in names
    assert "columns" in names


def test_create_tabular_ir_columns_axis_labels(tabular_ir, sample_columns):
    col_axis = next(ax for ax in tabular_ir.axes if ax.name == "columns")
    assert col_axis.labels == [c.name for c in sample_columns]


def test_create_tabular_ir_data_schema(tabular_ir, sample_columns, sample_rows):
    data = tabular_ir.data
    assert "columns" in data
    assert "rows" in data
    assert len(data["columns"]) == len(sample_columns)
    assert len(data["rows"]) == len(sample_rows)


# -- Round-trip tests --

def test_tabular_ir_round_trip_column_names(tabular_ir, sample_columns):
    """Column names survive a model_dump → reload cycle."""
    dumped = tabular_ir.model_dump(mode="json")
    col_names = [c["name"] for c in dumped["data"]["columns"]]
    assert col_names == [c.name for c in sample_columns]


def test_tabular_ir_round_trip_rows(tabular_ir, sample_rows):
    """Row values are preserved verbatim."""
    dumped = tabular_ir.model_dump(mode="json")
    assert dumped["data"]["rows"] == sample_rows


def test_tabular_ir_round_trip_units(tabular_ir):
    """Unit metadata is preserved."""
    dumped = tabular_ir.model_dump(mode="json")
    revenue_col = next(c for c in dumped["data"]["columns"] if c["name"] == "revenue")
    assert revenue_col["unit"] == "USD"


# -- RFC 6902 structured patch tests --

@pytest.fixture
def env():
    return OperatorEnv(seed=0, renderer_version="1.0", policy="strict", env_scope=_DEV_SCOPE)


def test_tabular_patch_replace_cell(tabular_ir, env):
    """RFC 6902 replace op updates a specific cell value."""
    doc = tabular_ir.model_dump(mode="json")
    ops = [{"op": "replace", "path": "/data/rows/0/revenue", "value": 1_300_000}]
    params = OperatorParams(name="patch", parameters={"delta": ops, "delta_type": "structured"})
    result = ApplyOperator().apply(doc, params, env)
    assert result["data"]["rows"][0]["revenue"] == 1_300_000


def test_tabular_patch_add_row(tabular_ir, env):
    """RFC 6902 add op appends a new row."""
    doc = tabular_ir.model_dump(mode="json")
    new_row = {"quarter": "Q3", "revenue": 1_700_000, "growth_pct": 0.30}
    ops = [{"op": "add", "path": "/data/rows/-", "value": new_row}]
    params = OperatorParams(name="patch", parameters={"delta": ops, "delta_type": "structured"})
    result = ApplyOperator().apply(doc, params, env)
    assert len(result["data"]["rows"]) == 3
    assert result["data"]["rows"][-1]["quarter"] == "Q3"


def test_tabular_patch_remove_column_metadata(tabular_ir, env):
    """RFC 6902 remove op strips optional column metadata."""
    doc = tabular_ir.model_dump(mode="json")
    # Remove format_spec from growth_pct column (index 2)
    ops = [{"op": "remove", "path": "/data/columns/2/format_spec"}]
    params = OperatorParams(name="patch", parameters={"delta": ops, "delta_type": "structured"})
    result = ApplyOperator().apply(doc, params, env)
    assert "format_spec" not in result["data"]["columns"][2]


def test_tabular_patch_is_non_destructive(tabular_ir, env):
    """Patching does not mutate the original IR."""
    doc = tabular_ir.model_dump(mode="json")
    original_revenue = doc["data"]["rows"][0]["revenue"]
    ops = [{"op": "replace", "path": "/data/rows/0/revenue", "value": 0}]
    params = OperatorParams(name="patch", parameters={"delta": ops, "delta_type": "structured"})
    ApplyOperator().apply(doc, params, env)
    # Original dict unchanged
    assert doc["data"]["rows"][0]["revenue"] == original_revenue


def test_tabular_patch_multi_op(tabular_ir, env):
    """Multiple ops applied in sequence produce correct final state."""
    doc = tabular_ir.model_dump(mode="json")
    ops = [
        {"op": "replace", "path": "/data/rows/0/revenue", "value": 9_999},
        {"op": "replace", "path": "/data/rows/1/growth_pct", "value": 0.99},
    ]
    params = OperatorParams(name="patch", parameters={"delta": ops, "delta_type": "structured"})
    result = ApplyOperator().apply(doc, params, env)
    assert result["data"]["rows"][0]["revenue"] == 9_999
    assert result["data"]["rows"][1]["growth_pct"] == 0.99
