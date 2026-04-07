# packages/ikam/tests/test_ir_mime_types.py
"""Tests for IR MIME type constants."""


def test_surface_mime_types_are_strings():
    from ikam.ir.mime_types import HEADING, PARAGRAPH, FORMULA_CELL, VALUE_CELL, TABLE_REGION, SLIDE_SHAPE, PDF_PAGE
    for m in [HEADING, PARAGRAPH, FORMULA_CELL, VALUE_CELL, TABLE_REGION, SLIDE_SHAPE, PDF_PAGE]:
        assert isinstance(m, str)
        assert "/" in m


def test_ir_mime_types_are_strings():
    from ikam.ir.mime_types import EXPRESSION_IR, CLAIM_IR, TABLE_IR, STYLE_IR
    for m in [EXPRESSION_IR, CLAIM_IR, TABLE_IR, STYLE_IR]:
        assert isinstance(m, str)
        assert "ikam" in m
        assert "+json" in m


def test_system_mime_types_are_strings():
    from ikam.ir.mime_types import RECONSTRUCTION_PROGRAM, VERIFICATION_RESULT
    assert "reconstruction" in RECONSTRUCTION_PROGRAM
    assert "verification" in VERIFICATION_RESULT


def test_no_duplicate_mime_types():
    from ikam.ir import mime_types
    all_values = [v for k, v in vars(mime_types).items() if not k.startswith("_") and isinstance(v, str)]
    assert len(all_values) == len(set(all_values)), f"Duplicate MIME types: {all_values}"
