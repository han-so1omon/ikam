from __future__ import annotations

from dataclasses import dataclass
from typing import Any


POLICY_VERSION = "v1"


@dataclass(frozen=True)
class BoundaryRule:
    min_boundaries: int
    max_chunk_chars: int
    require_full_coverage: bool = False


_RULES: dict[str, BoundaryRule] = {
    "text/markdown": BoundaryRule(min_boundaries=2, max_chunk_chars=1800, require_full_coverage=True),
    "text/plain": BoundaryRule(min_boundaries=2, max_chunk_chars=1800, require_full_coverage=True),
    "application/pdf": BoundaryRule(min_boundaries=1, max_chunk_chars=2200),
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet": BoundaryRule(min_boundaries=2, max_chunk_chars=3000),
    "application/vnd.openxmlformats-officedocument.presentationml.presentation": BoundaryRule(min_boundaries=1, max_chunk_chars=1600),
}


def _resolve_rule(mime_type: str) -> BoundaryRule:
    return _RULES.get(mime_type, BoundaryRule(min_boundaries=1, max_chunk_chars=2000))


def evaluate_boundary_quality(*, mime_type: str, boundary_count: int, max_chunk_chars: int, structural_coverage_ratio: float | None) -> tuple[str, str]:
    rule = _resolve_rule(mime_type)
    if boundary_count <= 0:
        return "failed", "no boundaries generated"
    if boundary_count < rule.min_boundaries:
        return "coarse", f"boundary_count={boundary_count} below minimum {rule.min_boundaries}"
    if max_chunk_chars > rule.max_chunk_chars:
        return "coarse", f"max_chunk_chars={max_chunk_chars} exceeds {rule.max_chunk_chars}"
    if rule.require_full_coverage:
        if structural_coverage_ratio is None:
            return "failed", "coverage ratio unavailable for strict text policy"
        if structural_coverage_ratio < 1.0:
            return "failed", f"structural_coverage_ratio={structural_coverage_ratio:.4f} must equal 1.0"
    return "good", "boundaries satisfy policy"


def build_boundary_diagnostic(metric: dict[str, Any]) -> dict[str, Any]:
    chunk_distribution = list(metric.get("chunk_distribution") or [])
    boundary_count = int(metric.get("boundary_count") or 0)
    max_chunk_chars = int(metric.get("max_chunk_chars") or 0)
    structural_coverage_ratio = metric.get("structural_coverage_ratio")
    if structural_coverage_ratio is not None:
        structural_coverage_ratio = float(structural_coverage_ratio)
    status, status_reason = evaluate_boundary_quality(
        mime_type=str(metric.get("mime_type") or "application/octet-stream"),
        boundary_count=boundary_count,
        max_chunk_chars=max_chunk_chars,
        structural_coverage_ratio=structural_coverage_ratio,
    )
    return {
        **metric,
        "policy_version": POLICY_VERSION,
        "status": status,
        "status_reason": status_reason,
        "chunk_distribution": chunk_distribution,
    }
