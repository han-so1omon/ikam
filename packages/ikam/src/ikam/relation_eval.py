"""Relation evaluator for V3 Fragment Algebra.

Per IKAM_FRAGMENT_ALGEBRA_V3.md §3.3-§3.4:
- eval(r, g, env) = out
- function_cas_id resolves to executable function spec
- g.slots bind named inputs to source fragment identities
- env is reproducibility context (may be empty)
- Deterministic: same slots + function => same output bytes
- Non-deterministic: environment metadata required for replay
- Evaluation outcome persisted as provenance event

References:
  - docs/ikam/IKAM_FRAGMENT_ALGEBRA_V3.md §2.2, §3.3, §3.4
  - docs/ikam/IKAM_MONOID_ALGEBRA_CONTRACT.md
"""

from __future__ import annotations

import datetime as dt
import uuid
from dataclasses import dataclass, field
from typing import Any, Callable, Optional

from ikam.fragments import (
    BindingGroup,
    Fragment,
    Relation,
    RELATION_MIME,
    is_relation_fragment,
)

try:
    from ikam.graph import _cas_hex
except ImportError:  # pragma: no cover
    import hashlib

    def _cas_hex(data: bytes) -> str:
        return hashlib.blake2b(data).hexdigest()


# Type alias for evaluation functions: (resolved_slots, environment) -> output_bytes
EvalFunction = Callable[[dict[str, str], dict[str, Any]], bytes]


def extract_relation(fragment: Fragment) -> Relation:
    """Extract and validate the Relation payload from a relation fragment.

    Raises ValueError if fragment is not a relation fragment.
    """
    if not is_relation_fragment(fragment):
        raise ValueError(
            f"Fragment (cas_id={fragment.cas_id}) is not a relation fragment "
            f"(mime_type={fragment.mime_type!r}, expected {RELATION_MIME!r})"
        )
    payload = fragment.value
    if isinstance(payload, dict):
        return Relation(**payload)
    if isinstance(payload, Relation):
        return payload
    raise ValueError(f"Cannot parse relation payload of type {type(payload)}")


def resolve_slots(binding_group: BindingGroup) -> dict[str, str]:
    """Resolve a binding group's slots to a {slot_name: fragment_id} dict."""
    return {sb.slot: sb.fragment_id for sb in binding_group.slots}


@dataclass
class EvaluationResult:
    """Result of evaluating a relation fragment for one binding group."""

    output_bytes: bytes
    output_cas_id: str
    relation_fragment_id: str
    invocation_id: str
    deterministic: bool = True
    environment: dict[str, Any] = field(default_factory=dict)
    timestamp: dt.datetime = field(
        default_factory=lambda: dt.datetime.now(dt.timezone.utc)
    )

    def to_provenance_event(self) -> dict[str, Any]:
        """Build a provenance event dict compatible with LatestOutputProjection."""
        return {
            "id": str(uuid.uuid4()),
            "fragment_id": self.relation_fragment_id,
            "action": "evaluated",
            "timestamp": self.timestamp,
            "metadata": {
                "invocation_id": self.invocation_id,
                "output_cas_id": self.output_cas_id,
                "environment": self.environment,
                "deterministic": self.deterministic,
            },
        }


def evaluate_relation(
    fragment: Fragment,
    *,
    invocation_id: str,
    function_registry: dict[str, EvalFunction],
    environment: dict[str, Any] | None = None,
    deterministic: bool = True,
) -> EvaluationResult:
    """Evaluate a relation fragment for a specific invocation.

    Args:
        fragment: V3 Fragment with RELATION_MIME
        invocation_id: which binding group to evaluate
        function_registry: maps function_cas_id -> callable
        environment: reproducibility context for non-deterministic functions
        deterministic: whether the function is deterministic

    Returns:
        EvaluationResult with output bytes and provenance event data

    Raises:
        ValueError: if invocation_id not found or function missing
    """
    rel = extract_relation(fragment)
    env = environment or {}

    # Find the binding group matching the invocation_id
    matching = [bg for bg in rel.binding_groups if bg.invocation_id == invocation_id]
    if not matching:
        available = [bg.invocation_id for bg in rel.binding_groups]
        raise ValueError(
            f"invocation_id {invocation_id!r} not found in relation binding groups. "
            f"Available: {available}"
        )

    binding_group = matching[0]
    slots = resolve_slots(binding_group)

    # Resolve and call the function
    fn_id = rel.function_cas_id
    if fn_id is None:
        raise ValueError("Cannot evaluate a pure-semantic relation (no function_cas_id)")
    fn = function_registry.get(fn_id)
    if fn is None:
        raise ValueError(f"Function {fn_id!r} not found in registry")

    output_bytes = fn(slots, env)
    output_cas_id = _cas_hex(output_bytes)

    return EvaluationResult(
        output_bytes=output_bytes,
        output_cas_id=output_cas_id,
        relation_fragment_id=fragment.cas_id or "",
        invocation_id=invocation_id,
        deterministic=deterministic,
        environment=env,
    )


__all__ = [
    "extract_relation",
    "resolve_slots",
    "evaluate_relation",
    "EvaluationResult",
    "EvalFunction",
]
