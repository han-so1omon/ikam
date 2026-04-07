"""Tests for ClaimIR and TableIR models."""
import json


from ikam.ir import ClaimIR, TableIR, ColumnDef


def test_claim_ir_basic():
    claim = ClaimIR(
        artifact_id="art:123",
        subject="Acme Corp",
        predicate="has-revenue",
        object="$10M",
    )
    assert claim.subject == "Acme Corp"
    assert claim.predicate == "has-revenue"
    assert claim.object == "$10M"
    assert claim.confidence == 1.0
    assert claim.qualifiers == {}


def test_claim_ir_with_qualifiers():
    claim = ClaimIR(
        artifact_id="art:123",
        subject="Acme Corp",
        predicate="has-revenue",
        object="$10M",
        confidence=0.85,
        qualifiers={"period": "FY2025", "source": "10-10K filing"},
    )
    assert claim.qualifiers["period"] == "FY2025"
    # Round-trip through JSON
    data = claim.model_dump(mode="json")
    restored = ClaimIR.model_validate(data)
    assert restored == claim


def test_table_ir_basic():
    tbl = TableIR(
        artifact_id="art:123",
        columns=[
            ColumnDef(name="date", dtype="date"),
            ColumnDef(name="revenue", dtype="numeric", unit="USD"),
        ],
        rows=[
            {"date": "2026-01", "revenue": 10000},
            {"date": "2026-02", "revenue": 12000},
        ],
    )
    assert len(tbl.columns) == 2
    assert len(tbl.rows) == 2
    assert tbl.columns[1].unit == "USD"


def test_table_ir_roundtrip():
    tbl = TableIR(
        artifact_id="art:123",
        columns=[ColumnDef(name="x", dtype="text")],
        rows=[{"x": "hello"}],
    )
    data = tbl.model_dump(mode="json")
    restored = TableIR.model_validate(data)
    assert restored == tbl


def test_all_ir_models_importable_from_package():
    assert ClaimIR is not None
    assert TableIR is not None
