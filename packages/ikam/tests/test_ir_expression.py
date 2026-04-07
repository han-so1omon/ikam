"""Tests for ExpressionIR model (ir/core.py)."""
from ikam.ir import (
    OpType, OpAST, canonicalize_ast, compute_shape_hash, fold_constants
)


def test_expression_ir_imports_from_new_location():
    assert OpType.ADD == "ADD"


def test_expression_ir_available_from_ir_package():
    assert OpType.MUL == "MUL"


def test_canonicalize_commutative():
    ast = OpAST(op_type=OpType.ADD, operands=["b", "a"])
    canonical = canonicalize_ast(ast)
    assert canonical.operands == ["a", "b"]


def test_compute_shape_hash_deterministic():
    ast1 = OpAST(op_type=OpType.ADD, operands=["a", "b"])
    ast2 = OpAST(op_type=OpType.ADD, operands=["b", "a"])
    assert compute_shape_hash(ast1) == compute_shape_hash(ast2)
