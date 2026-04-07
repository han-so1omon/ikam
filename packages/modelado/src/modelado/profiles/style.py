"""Style Subgraph Profile (D20) — Visual styling as a computational subgraph.

Styling is not a flat property bag. It is a structured derivation:
  - StyleRule (StructuredDataIR, profile STYLE_SUBGRAPH_V1): layout and rule table
  - Computed values (ExpressionIR): one per resolved property, with output_units
  - Design constraints (PropositionIR): NORMATIVE modality, evidence_refs → ExpressionIR

Usage:
    subgraph = create_style_subgraph(
        artifact_id="theme:abc",
        theme_artifact_id="theme:abc",
        rules=[StyleRule(property="font-size", raw_value="16px", output_units="pt")],
        constraints=[...],   # optional PropositionIR list
    )
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import List, Optional

from pydantic import Field

from ikam.ir import (
    ExpressionIR,
    OpAST,
    OpType,
    PropositionIR,
    StructuredDataIR,
)
from modelado.profiles import STYLE_SUBGRAPH_V1


# ---------------------------------------------------------------------------
# Domain model
# ---------------------------------------------------------------------------

@dataclass
class StyleRule:
    """A single visual design rule with its resolved expression."""
    property: str           # e.g. "font-size", "color"
    raw_value: str          # e.g. "16px", "#1a1a1a"
    output_units: str = ""  # e.g. "pt", "EMU", "px" — empty means unitless


@dataclass
class StyleSubgraph:
    """Style Subgraph (D20): rule table + computed expressions + design constraints."""
    rule_table: StructuredDataIR
    expressions: List[ExpressionIR] = field(default_factory=list)
    constraints: List[PropositionIR] = field(default_factory=list)


# ---------------------------------------------------------------------------
# Factory
# ---------------------------------------------------------------------------

def create_style_subgraph(
    artifact_id: str,
    theme_artifact_id: str,
    rules: List[StyleRule],
    constraints: Optional[List[PropositionIR]] = None,
) -> StyleSubgraph:
    """Build a Style Subgraph from a list of StyleRules (D20).

    Each rule produces one ExpressionIR (the computed property value).
    The rule table (StructuredDataIR) owns shared identity via theme_artifact_id.
    """
    expressions: List[ExpressionIR] = []
    for rule in rules:
        expr = ExpressionIR(
            artifact_id=artifact_id,
            ast=OpAST(op_type=OpType.LOAD, operands=[rule.raw_value]),
            output_units=rule.output_units or None,
        )
        expressions.append(expr)

    rule_table = StructuredDataIR(
        artifact_id=theme_artifact_id,
        profile=STYLE_SUBGRAPH_V1,
        data=[
            {
                "property": rule.property,
                "raw_value": rule.raw_value,
                "output_units": rule.output_units,
            }
            for rule in rules
        ],
    )

    return StyleSubgraph(
        rule_table=rule_table,
        expressions=expressions,
        constraints=constraints or [],
    )
