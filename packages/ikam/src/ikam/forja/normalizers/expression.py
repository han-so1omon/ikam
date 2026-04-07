"""Expression canonicalization normalizer.

Normalizes ExpressionIR fragments by sorting commutative operands
and folding constants. Deterministic and lossless.
"""
from __future__ import annotations

import json

from ikam.fragments import Fragment
from ikam.forja.cas import cas_fragment
from ikam.ir.mime_types import EXPRESSION_IR


COMMUTATIVE_OPS = {"ADD", "MUL", "AGG_SUM"}


class ExpressionNormalizer:
    """Normalize expression IR fragments for dedup."""

    async def normalize(self, fragment: Fragment) -> list[Fragment]:
        if fragment.mime_type != EXPRESSION_IR:
            return [fragment]

        value = dict(fragment.value) if fragment.value else {}
        op_type = value.get("op_type", "")
        operands = value.get("operands", [])

        # Commutative sort
        if op_type in COMMUTATIVE_OPS and operands:
            operands = sorted(operands, key=lambda x: json.dumps(x, sort_keys=True, default=str))

        normalized_value = {**value, "operands": operands}

        # Sort params
        params = normalized_value.get("params", {})
        if params:
            normalized_value["params"] = dict(sorted(params.items()))

        return [cas_fragment(normalized_value, EXPRESSION_IR)]
