"""Tests for expression canonicalization normalizer."""
import asyncio


def test_canonicalize_commutative_add():
    from ikam.forja.normalizers.expression import ExpressionNormalizer
    from ikam.fragments import Fragment
    from ikam.ir.mime_types import EXPRESSION_IR

    frag = Fragment(
        value={
            "op_type": "ADD",
            "operands": ["b_var", "a_var"],
            "params": {},
        },
        mime_type=EXPRESSION_IR,
    )

    normalizer = ExpressionNormalizer()
    results = asyncio.run(normalizer.normalize(frag))

    assert len(results) >= 1
    normalized = results[0]
    assert normalized.mime_type == EXPRESSION_IR
    # Commutative sort: a_var before b_var
    assert normalized.value["operands"] == ["a_var", "b_var"]


def test_already_canonical_no_change():
    from ikam.forja.normalizers.expression import ExpressionNormalizer
    from ikam.fragments import Fragment
    from ikam.ir.mime_types import EXPRESSION_IR

    frag = Fragment(
        value={"op_type": "ADD", "operands": ["a", "b"], "params": {}},
        mime_type=EXPRESSION_IR,
    )

    normalizer = ExpressionNormalizer()
    results = asyncio.run(normalizer.normalize(frag))
    # Already canonical — should return same structure
    assert results[0].value["operands"] == ["a", "b"]


def test_non_expression_passthrough():
    """Non-expression fragments pass through unchanged."""
    from ikam.forja.normalizers.expression import ExpressionNormalizer
    from ikam.fragments import Fragment

    frag = Fragment(value={"text": "hello"}, mime_type="text/ikam-paragraph")

    normalizer = ExpressionNormalizer()
    results = asyncio.run(normalizer.normalize(frag))
    assert len(results) == 1
    assert results[0] is frag  # Same object — not mutated


def test_params_sorted():
    """Params dict gets sorted for canonical form."""
    from ikam.forja.normalizers.expression import ExpressionNormalizer
    from ikam.fragments import Fragment
    from ikam.ir.mime_types import EXPRESSION_IR

    frag = Fragment(
        value={
            "op_type": "MUL",
            "operands": ["x", "y"],
            "params": {"z_key": 1, "a_key": 2},
        },
        mime_type=EXPRESSION_IR,
    )

    normalizer = ExpressionNormalizer()
    results = asyncio.run(normalizer.normalize(frag))
    keys = list(results[0].value["params"].keys())
    assert keys == ["a_key", "z_key"]
