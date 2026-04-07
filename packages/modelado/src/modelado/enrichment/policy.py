from __future__ import annotations

from dataclasses import dataclass
from enum import Enum


class LaneMode(str, Enum):
    EXPLORE_FAST = "explore-fast"
    EXPLORE_GRAPH = "explore-graph"
    REVIEW_CANDIDATE = "review-candidate"
    COMMIT_STRICT = "commit-strict"
    COMMIT_MERGE = "commit-merge"


class ReuseScope(str, Enum):
    ARTIFACT = "artifact"
    CASE = "case"
    WORKSPACE = "workspace"
    CROSS_CASE = "cross-case"


class DedupMode(str, Enum):
    CAS_STRICT = "cas_strict"
    SEMANTIC = "semantic"
    HYBRID = "hybrid"


@dataclass(frozen=True)
class ReusePolicy:
    scope: ReuseScope
    mode: LaneMode
    dedup_mode: DedupMode
    explicit_merge_approval: bool = False
    within_case_primary: bool = True
    cross_case_appendix_only: bool = True


def _is_commit_mode(mode: LaneMode) -> bool:
    return mode in (LaneMode.COMMIT_STRICT, LaneMode.COMMIT_MERGE)


def is_commit_eligible(
    mode: LaneMode,
    evidence_ok: bool,
    provenance_ok: bool,
    *,
    deterministic_identity_ok: bool = True,
    deterministic_idempotency_ok: bool = True,
) -> bool:
    if not _is_commit_mode(mode):
        return True
    return (
        evidence_ok
        and provenance_ok
        and deterministic_identity_ok
        and deterministic_idempotency_ok
    )


def allows_cross_case_reuse(
    mode: LaneMode,
    scope: ReuseScope,
    *,
    explicit_merge_approval: bool = False,
) -> bool:
    if scope is not ReuseScope.CROSS_CASE:
        return False
    if mode is not LaneMode.COMMIT_MERGE:
        return False
    return explicit_merge_approval


def dedup_is_policy_compliant(mode: LaneMode, dedup_mode: DedupMode) -> bool:
    if _is_commit_mode(mode):
        return dedup_mode in (DedupMode.CAS_STRICT, DedupMode.HYBRID)
    return True
