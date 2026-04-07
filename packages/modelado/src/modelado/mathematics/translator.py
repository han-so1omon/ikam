"""Semantic Math Translator.

Provides functions to:
1. Parse natural language descriptions of formulas into SymPy expressions.
2. Convert free‑form textual math (LaTeX, inline arithmetic) into symbolic form.
3. Export expressions to LaTeX, MathML, and Python callables.

Design Goals:
- Deterministic, safe parsing (no eval of arbitrary input).
- Clear error reporting with structured result objects.
- Extensible mapping dictionary for domain terms ("revenue", "growth rate") → symbols.

NOTE: This initial implementation focuses on a constrained subset suitable for MVP.
Future improvements can incorporate a small language model or pattern library
for more flexible natural language understanding.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Dict, Optional, Callable, Any, Tuple

import re
import sympy as sp

# Core symbol registry — extend as needed
DEFAULT_SYMBOLS: Dict[str, sp.Symbol] = {
    "revenue": sp.Symbol("R"),
    "cost": sp.Symbol("C"),
    "profit": sp.Symbol("P"),
    "growth_rate": sp.Symbol("g"),
    "time": sp.Symbol("t"),
    "customers": sp.Symbol("N"),
    "price": sp.Symbol("p"),
    "quantity": sp.Symbol("q"),
}


@dataclass
class TranslationResult:
    """Structured outcome of a translation attempt."""

    success: bool
    expr: Optional[sp.Expr]
    message: str
    symbols: Dict[str, sp.Symbol]

    def to_dict(self) -> Dict[str, Any]:  # convenience for JSON responses
        return {
            "success": self.success,
            "message": self.message,
            "expr": str(self.expr) if self.expr is not None else None,
            "latex": to_latex(self.expr) if self.expr is not None else None,
            "mathml": to_mathml(self.expr) if self.expr is not None else None,
            "symbols": {k: str(v) for k, v in self.symbols.items()},
        }


NL_PATTERN_RULES: Tuple[Tuple[re.Pattern, str], ...] = (
    # profit equals revenue minus cost
    (re.compile(r"profit\s+(is|=|equals)\s+revenue\s+minus\s+cost", re.I), "Eq(P, R - C)"),
    # revenue equals price times quantity
    (re.compile(r"revenue\s+(is|=|equals)\s+price\s+times\s+quantity", re.I), "Eq(R, p * q)"),
    # growth rate proportional to time (simple linear growth example)
    (re.compile(r"growth rate\s+(is|=|equals)\s+([0-9.]+)\s*%", re.I), lambda m: f"g = {float(m.group(2))/100.0}"),
)


SAFE_EXPR_PATTERN = re.compile(r"^[0-9a-zA-Z_=\s+\-*/().,^]+$")


def _apply_nl_rules(text: str) -> Optional[str]:
    for pattern, replacement in NL_PATTERN_RULES:
        match = pattern.search(text)
        if match:
            if callable(replacement):  # dynamic builder
                return replacement(match)
            return replacement
    return None


def to_expr_from_text(expr_text: str, symbols: Dict[str, sp.Symbol] | None = None) -> TranslationResult:
    """Parse a textual math expression (already in math form) into SymPy.

    Supports a restricted grammar (alphanumerics, operators, parentheses).
    """
    if symbols is None:
        symbols = DEFAULT_SYMBOLS

    cleaned = expr_text.strip()
    if not cleaned:
        return TranslationResult(False, None, "Empty expression", symbols)

    if not SAFE_EXPR_PATTERN.match(cleaned):
        return TranslationResult(False, None, "Expression contains unsupported characters", symbols)

    # Replace word-based names with symbol names if present
    for name, sym in symbols.items():
        # simple whole word replacement
        cleaned = re.sub(rf"\b{name}\b", str(sym), cleaned, flags=re.I)

    try:
        # Fast-path for simple binary difference "X - Y" to preserve intuitive ordering
        diff_match = re.fullmatch(r"([A-Za-z]\w*)\s*-\s*([A-Za-z]\w*)", cleaned)
        if diff_match:
            left_name, right_name = diff_match.group(1), diff_match.group(2)
            # Resolve to existing symbols if present (case-insensitive match), else create new ones
            def _resolve(name: str) -> sp.Symbol:
                for s in symbols.values():
                    if s.name.lower() == name.lower():
                        return s
                return sp.Symbol(name)
            left_sym = _resolve(left_name)
            right_sym = _resolve(right_name)
            expr = left_sym - right_sym
        else:
            expr = sp.sympify(cleaned, locals={str(s): s for s in symbols.values()})
            # Minor reformat: if expression is an Add of two terms and second is negative, rewrite as subtraction
            if isinstance(expr, sp.Add):
                terms = list(expr.args)
                if len(terms) == 2:
                    a, b = terms
                    # If one term is negative and the other positive, render as positive - positive
                    if a.is_positive and b.is_negative:
                        expr = a - (-b)
                    elif b.is_positive and a.is_negative:
                        expr = b - (-a)
        return TranslationResult(True, expr, "Parsed expression", symbols)
    except Exception as e:  # noqa: BLE001 broad for user feedback
        return TranslationResult(False, None, f"Parse error: {e}", symbols)


def translate_nl(text: str, symbols: Dict[str, sp.Symbol] | None = None) -> TranslationResult:
    """Translate natural language description of a formula to a SymPy expression.

    Strategy:
      1. Attempt pattern rules (deterministic mappings) → algebraic string.
      2. If no rule matches, attempt direct parsing of text (assuming user already gave math).
    """
    if symbols is None:
        symbols = DEFAULT_SYMBOLS

    original = text.strip()
    if not original:
        return TranslationResult(False, None, "Empty input", symbols)

    # Try rule-based translation
    rule_expr = _apply_nl_rules(original)
    if rule_expr:
        return to_expr_from_text(rule_expr, symbols)

    # Fallback: treat the input itself as an expression (allow shorthand equality)
    # Convert simple "A = B" into SymPy Eq(A, B) for convenience
    if re.search(r"=", original) and not original.strip().startswith("Eq("):
        parts = original.split("=")
        if len(parts) == 2:
            left, right = parts[0].strip(), parts[1].strip()
            return to_expr_from_text(f"Eq({left}, {right})", symbols)

    # Otherwise, parse as-is
    return to_expr_from_text(original, symbols)


def to_latex(expr: Optional[sp.Expr]) -> Optional[str]:  # trivial wrappers
    if expr is None:
        return None
    return sp.latex(expr)


def to_mathml(expr: Optional[sp.Expr]) -> Optional[str]:
    if expr is None:
        return None
    try:
        return sp.printing.mathml(expr)  # type: ignore[attr-defined]
    except Exception:
        return None


def to_python_function(expr: sp.Expr, arg_symbols: Optional[list[sp.Symbol]] = None) -> Callable:
    """Compile a SymPy expression into a pure Python callable.

    Args:
        expr: Symbolic expression
        arg_symbols: Ordered list of symbols representing positional args
    """
    if arg_symbols is None:
        arg_symbols = sorted(expr.free_symbols, key=lambda s: s.name)
    f = sp.lambdify(arg_symbols, expr, modules=["math"])  # safe math backend

    def wrapper(**kwargs: float) -> float:
        values = []
        for sym in arg_symbols:
            if sym.name not in kwargs:
                raise ValueError(f"Missing value for symbol {sym}")
            values.append(kwargs[sym.name])
        return float(f(*values))

    wrapper.__doc__ = f"Evaluates {expr} with arguments {[s.name for s in arg_symbols]}"
    return wrapper


def evaluate_expr(expr: sp.Expr, **values: float) -> float:
    """Convenience evaluation with keyword arguments."""
    f = to_python_function(expr, list(expr.free_symbols))
    return f(**values)


__all__ = [
    "TranslationResult",
    "translate_nl",
    "to_expr_from_text",
    "to_latex",
    "to_mathml",
    "to_python_function",
    "evaluate_expr",
]

# --- Additional transformation utilities ---

def simplify_expr(expr: sp.Expr) -> sp.Expr:
    """Return a simplified form of the expression using SymPy's heuristics."""
    return sp.simplify(expr)


def differentiate_expr(expr: sp.Expr, var: str) -> sp.Expr:
    """Differentiate expression with respect to variable name `var`."""
    symbol = sp.Symbol(var)
    return sp.diff(expr, symbol)


__all__ += [
    "simplify_expr",
    "differentiate_expr",
]
