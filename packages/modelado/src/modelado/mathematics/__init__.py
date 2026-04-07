"""Mathematics module: Natural language ↔ symbolic formula translation.

Includes natural language → symbolic translation, and conversions between
SymPy expressions and common formats (LaTeX, MathML, Python code).

Example:
    Translate a sentence to a symbolic expression and render LaTeX::

        from modelado.mathematics import translate_nl, to_latex

        result = translate_nl("Revenue equals price times quantity")
        assert result.expr is not None
        latex = to_latex(result.expr)
        assert "\\times" in latex
"""

from .translator import (
    TranslationResult,
    translate_nl,
    to_expr_from_text,
    to_latex,
    to_mathml,
    to_python_function,
    evaluate_expr,
    simplify_expr,
    differentiate_expr,
)

__all__ = [
    "TranslationResult",
    "translate_nl",
    "to_expr_from_text",
    "to_latex",
    "to_mathml",
    "to_python_function",
    "evaluate_expr",
    "simplify_expr",
    "differentiate_expr",
]
