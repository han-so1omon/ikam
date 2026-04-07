from modelado.enrichment.policy import (
    DedupMode,
    LaneMode,
    ReuseScope,
    allows_cross_case_reuse,
    dedup_is_policy_compliant,
    is_commit_eligible,
)


def test_exploration_modes_allow_non_deterministic_outputs() -> None:
    assert is_commit_eligible(LaneMode.EXPLORE_FAST, evidence_ok=False, provenance_ok=False)
    assert is_commit_eligible(LaneMode.EXPLORE_GRAPH, evidence_ok=False, provenance_ok=False)
    assert is_commit_eligible(LaneMode.REVIEW_CANDIDATE, evidence_ok=False, provenance_ok=False)
    assert dedup_is_policy_compliant(LaneMode.EXPLORE_FAST, DedupMode.SEMANTIC)


def test_commit_modes_require_deterministic_identity_surfaces() -> None:
    assert not is_commit_eligible(LaneMode.COMMIT_STRICT, evidence_ok=True, provenance_ok=False)
    assert not is_commit_eligible(
        LaneMode.COMMIT_MERGE,
        evidence_ok=True,
        provenance_ok=True,
        deterministic_identity_ok=False,
    )
    assert not dedup_is_policy_compliant(LaneMode.COMMIT_STRICT, DedupMode.SEMANTIC)
    assert dedup_is_policy_compliant(LaneMode.COMMIT_STRICT, DedupMode.CAS_STRICT)


def test_reuse_policy_supports_scope_mode_and_explicit_merge() -> None:
    assert not allows_cross_case_reuse(LaneMode.EXPLORE_GRAPH, ReuseScope.CROSS_CASE)
    assert not allows_cross_case_reuse(LaneMode.COMMIT_STRICT, ReuseScope.CROSS_CASE)
    assert not allows_cross_case_reuse(LaneMode.COMMIT_MERGE, ReuseScope.CROSS_CASE)
    assert allows_cross_case_reuse(
        LaneMode.COMMIT_MERGE,
        ReuseScope.CROSS_CASE,
        explicit_merge_approval=True,
    )

    assert not allows_cross_case_reuse(LaneMode.COMMIT_MERGE, ReuseScope.ARTIFACT)
    assert not allows_cross_case_reuse(LaneMode.COMMIT_MERGE, ReuseScope.CASE)
    assert not allows_cross_case_reuse(LaneMode.COMMIT_MERGE, ReuseScope.WORKSPACE)
