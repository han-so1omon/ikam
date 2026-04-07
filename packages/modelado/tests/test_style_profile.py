"""Style Subgraph Profile Tests (D20, Plan D Task 6).

Validates create_style_subgraph() structure, ExpressionIR outputs,
PropositionIR design constraints, round-trip serialization, and
RFC 6902 patching of the rule table via ApplyOperator.
"""
import pytest

from ikam.ir import StructuredDataIR
from ikam.ir.core import EvidenceRef, ExpressionIR, Modality, OpType, PropositionIR
from modelado.environment_scope import EnvironmentScope
from modelado.operators.core import OperatorEnv, OperatorParams
from modelado.operators.monadic import ApplyOperator
from modelado.profiles import STYLE_SUBGRAPH_V1
from modelado.profiles.reasoning import REASONING_V1
from modelado.profiles.style import StyleRule, StyleSubgraph, create_style_subgraph


ARTIFACT_ID = "artifact:style-001"
THEME_ID = "theme:brand-v1"

_DEV_SCOPE = EnvironmentScope(ref="refs/heads/run/run/test")

SAMPLE_RULES = [
    StyleRule(property="font-size", raw_value="16px", output_units="pt"),
    StyleRule(property="color", raw_value="#1a1a1a"),
    StyleRule(property="padding", raw_value="8px 16px", output_units="EMU"),
]


def _env() -> OperatorEnv:
    return OperatorEnv(seed=0, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)


# ---------------------------------------------------------------------------
# StyleRule dataclass
# ---------------------------------------------------------------------------

def test_style_rule_fields():
    rule = StyleRule(property="font-size", raw_value="16px", output_units="pt")
    assert rule.property == "font-size"
    assert rule.raw_value == "16px"
    assert rule.output_units == "pt"


def test_style_rule_default_output_units():
    rule = StyleRule(property="color", raw_value="#fff")
    assert rule.output_units == ""


# ---------------------------------------------------------------------------
# StyleSubgraph structure
# ---------------------------------------------------------------------------

def test_create_style_subgraph_returns_style_subgraph():
    sg = create_style_subgraph(ARTIFACT_ID, THEME_ID, SAMPLE_RULES)
    assert isinstance(sg, StyleSubgraph)


def test_rule_table_is_structured_data_ir():
    sg = create_style_subgraph(ARTIFACT_ID, THEME_ID, SAMPLE_RULES)
    assert isinstance(sg.rule_table, StructuredDataIR)


def test_rule_table_profile():
    sg = create_style_subgraph(ARTIFACT_ID, THEME_ID, SAMPLE_RULES)
    assert sg.rule_table.profile == STYLE_SUBGRAPH_V1


def test_rule_table_artifact_id_is_theme():
    """Rule table is owned by the shared theme artifact (D20)."""
    sg = create_style_subgraph(ARTIFACT_ID, THEME_ID, SAMPLE_RULES)
    assert sg.rule_table.artifact_id == THEME_ID


def test_rule_table_data_contains_all_rules():
    sg = create_style_subgraph(ARTIFACT_ID, THEME_ID, SAMPLE_RULES)
    assert len(sg.rule_table.data) == len(SAMPLE_RULES)
    assert sg.rule_table.data[0]["property"] == "font-size"
    assert sg.rule_table.data[1]["property"] == "color"


# ---------------------------------------------------------------------------
# ExpressionIR — one per rule
# ---------------------------------------------------------------------------

def test_expressions_count_matches_rules():
    sg = create_style_subgraph(ARTIFACT_ID, THEME_ID, SAMPLE_RULES)
    assert len(sg.expressions) == len(SAMPLE_RULES)


def test_expressions_are_expression_ir():
    sg = create_style_subgraph(ARTIFACT_ID, THEME_ID, SAMPLE_RULES)
    for expr in sg.expressions:
        assert isinstance(expr, ExpressionIR)


def test_expression_output_units_preserved():
    sg = create_style_subgraph(ARTIFACT_ID, THEME_ID, SAMPLE_RULES)
    # font-size → "pt"
    assert sg.expressions[0].output_units == "pt"
    # padding → "EMU"
    assert sg.expressions[2].output_units == "EMU"


def test_expression_no_units_is_none():
    """Empty output_units string collapses to None on ExpressionIR."""
    sg = create_style_subgraph(ARTIFACT_ID, THEME_ID, SAMPLE_RULES)
    # color has no units
    assert sg.expressions[1].output_units is None


def test_expression_ast_op_type_load():
    sg = create_style_subgraph(ARTIFACT_ID, THEME_ID, SAMPLE_RULES)
    for expr in sg.expressions:
        assert expr.ast.op_type == OpType.LOAD


# ---------------------------------------------------------------------------
# PropositionIR constraints (optional)
# ---------------------------------------------------------------------------

def test_no_constraints_by_default():
    sg = create_style_subgraph(ARTIFACT_ID, THEME_ID, SAMPLE_RULES)
    assert sg.constraints == []


def test_constraints_passed_through():
    constraint = PropositionIR(
        artifact_id=ARTIFACT_ID,
        profile=REASONING_V1,
        statement={"subject": "font-size", "predicate": "must-be", "object": "≤24pt"},
        modality=Modality.NORMATIVE,
        evidence_refs=[EvidenceRef(fragment_id="frag:expr-font-size")],
    )
    sg = create_style_subgraph(ARTIFACT_ID, THEME_ID, SAMPLE_RULES, constraints=[constraint])
    assert len(sg.constraints) == 1
    assert sg.constraints[0].modality == Modality.NORMATIVE


# ---------------------------------------------------------------------------
# Round-trip: rule table serialization
# ---------------------------------------------------------------------------

def test_rule_table_round_trip():
    sg = create_style_subgraph(ARTIFACT_ID, THEME_ID, SAMPLE_RULES)
    dumped = sg.rule_table.model_dump(mode="json")
    restored = StructuredDataIR.model_validate(dumped)
    assert restored.profile == STYLE_SUBGRAPH_V1
    assert restored.artifact_id == THEME_ID
    assert len(restored.data) == len(SAMPLE_RULES)
    assert restored.data[0]["raw_value"] == "16px"


# ---------------------------------------------------------------------------
# ApplyOperator: RFC 6902 patching of the rule table
# ---------------------------------------------------------------------------

def test_apply_operator_replace_rule_value():
    sg = create_style_subgraph(ARTIFACT_ID, THEME_ID, SAMPLE_RULES)
    fragment = sg.rule_table.model_dump(mode="json")

    op = ApplyOperator()
    patch = [{"op": "replace", "path": "/data/0/raw_value", "value": "18px"}]
    result = op.apply(
        fragment,
        OperatorParams(name="patch", parameters={"delta": patch, "delta_type": "structured"}),
        _env(),
    )
    assert result["data"][0]["raw_value"] == "18px"
    # Other rows unchanged
    assert result["data"][1]["property"] == "color"


def test_apply_operator_add_new_rule():
    sg = create_style_subgraph(ARTIFACT_ID, THEME_ID, SAMPLE_RULES)
    fragment = sg.rule_table.model_dump(mode="json")

    new_rule = {"property": "margin", "raw_value": "0 auto", "output_units": ""}
    op = ApplyOperator()
    patch = [{"op": "add", "path": "/data/-", "value": new_rule}]
    result = op.apply(
        fragment,
        OperatorParams(name="patch", parameters={"delta": patch, "delta_type": "structured"}),
        _env(),
    )
    assert len(result["data"]) == len(SAMPLE_RULES) + 1
    assert result["data"][-1]["property"] == "margin"


def test_apply_operator_non_destructive():
    sg = create_style_subgraph(ARTIFACT_ID, THEME_ID, SAMPLE_RULES)
    fragment = sg.rule_table.model_dump(mode="json")
    original_value = fragment["data"][0]["raw_value"]

    op = ApplyOperator()
    patch = [{"op": "replace", "path": "/data/0/raw_value", "value": "99px"}]
    op.apply(
        fragment,
        OperatorParams(name="patch", parameters={"delta": patch, "delta_type": "structured"}),
        _env(),
    )
    # Original dict must be unchanged
    assert fragment["data"][0]["raw_value"] == original_value
