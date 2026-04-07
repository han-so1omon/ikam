from __future__ import annotations

from pathlib import Path

from ikam.ir.core import EvidenceRef, ExpressionIR, OpAST, OpType, PropositionIR, StructuredDataIR
from modelado.profiles import STYLE_SUBGRAPH_V1


def test_core_ir_models_exist_and_roundtrip() -> None:
    expr = ExpressionIR(
        artifact_id="art:expr-1",
        ast=OpAST(op_type=OpType.ADD, operands=[1, 2]),
        output_units="USD",
    )
    expr_dump = expr.model_dump(mode="json")
    assert ExpressionIR.model_validate(expr_dump) == expr

    prop = PropositionIR(
        artifact_id="art:prop-1",
        profile="modelado/reasoning@1",
        statement={"subject": "revenue", "predicate": "increased", "object": "q/q"},
        evidence_refs=[EvidenceRef(fragment_id="frag:1")],
    )
    prop_dump = prop.model_dump(mode="json")
    assert PropositionIR.model_validate(prop_dump) == prop

    data = StructuredDataIR(
        artifact_id="art:data-1",
        profile="modelado/tabular@1",
        shape=[1, 1],
        data=[[1]],
    )
    data_dump = data.model_dump(mode="json")
    assert StructuredDataIR.model_validate(data_dump) == data


def test_core_ir_models_share_required_base_fields() -> None:
    required = {"artifact_id", "scope_id", "provenance_id", "lsn"}
    assert required.issubset(ExpressionIR.model_fields.keys())
    assert required.issubset(PropositionIR.model_fields.keys())
    assert required.issubset(StructuredDataIR.model_fields.keys())


def test_style_subgraph_profile_registered_without_style_v1() -> None:
    import modelado.profiles as profiles

    assert STYLE_SUBGRAPH_V1 == "modelado/style-subgraph@1"
    assert hasattr(profiles, "STYLE_SUBGRAPH_V1")
    assert not hasattr(profiles, "STYLE_V1")


def test_style_ir_symbol_not_exported_from_ikam_ir() -> None:
    import ikam.ir as ir

    assert not hasattr(ir, "StyleIR")


def test_style_ir_symbol_not_present_in_ikam_ir_core_module() -> None:
    import ikam.ir.core as ir_core

    assert not hasattr(ir_core, "StyleIR")


def test_legacy_ir_models_test_file_has_no_style_ir_mentions() -> None:
    root = Path(__file__).resolve().parents[3]
    legacy_test_file = root / "packages" / "ikam" / "tests" / "test_ir_models.py"
    source = legacy_test_file.read_text(encoding="utf-8")
    assert "StyleIR" not in source
