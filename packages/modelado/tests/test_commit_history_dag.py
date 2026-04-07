from __future__ import annotations

from modelado.environment_scope import EnvironmentScope
from modelado.operators.commit import CommitOperator
from modelado.operators.core import OperatorEnv, OperatorParams


_DEV_SCOPE = EnvironmentScope(ref="refs/heads/run/run/test")


def _env(history: dict) -> OperatorEnv:
    return OperatorEnv(
        seed=11,
        renderer_version="1",
        policy="strict",
        env_scope=_DEV_SCOPE,
        history=history,
    )


def _params(
    *,
    ref: str = "refs/heads/main",
    parents: list[str] | None = None,
    target_ref: str | None = None,
    promoted_fragment_ids: list[str] | None = None,
) -> OperatorParams:
    return OperatorParams(
        name="commit",
        parameters={
            "commit_policy": "semantic_relations_only",
            "mapping_mode": "semantic_relations_only",
            "include_evidence_surface_fragments": False,
            "ref": ref,
            "target_ref": target_ref or ref,
            "parents": parents or [],
            "promoted_fragment_ids": promoted_fragment_ids or ["prop-1", "semantic-subgraph-1"],
            "proposition_fragments": [{"id": "prop-1"}],
            "semantic_subgraph_snapshot": {"id": "semantic-subgraph-1"},
            "community_reports": [{"id": "report-1"}],
            "surface_fragments": [{"id": "surface-1"}],
        },
    )


def test_commit_entries_use_structured_data_ir_profile() -> None:
    history: dict = {}
    result = CommitOperator().apply(None, _params(), _env(history))

    entry = result["commit_entry"]
    assert entry["mime_type"] == "application/ikam-structured-data+json"
    assert entry["profile"] == "modelado/commit-entry@1"


def test_commit_entry_supports_merge_parents() -> None:
    history: dict = {}
    result = CommitOperator().apply(
        None,
        _params(parents=["commit-a", "commit-b"]),
        _env(history),
    )
    assert result["commit_entry"]["content"]["parents"] == ["commit-a", "commit-b"]


def test_commit_entry_includes_target_ref_and_promoted_fragment_ids() -> None:
    history: dict = {}
    result = CommitOperator().apply(
        None,
        _params(
            ref="refs/heads/run/run-123",
            target_ref="refs/heads/main",
            promoted_fragment_ids=["prop-1", "report-1"],
        ),
        _env(history),
    )

    assert result["target_ref"] == "refs/heads/main"
    assert result["promoted_fragment_ids"] == ["prop-1", "report-1"]
    assert result["commit_entry"]["content"]["target_ref"] == "refs/heads/main"
    assert result["commit_entry"]["content"]["promoted_fragment_ids"] == ["prop-1", "report-1"]


def test_ref_heads_are_stored_per_ref_not_shared_blob() -> None:
    history: dict = {}
    op = CommitOperator()

    main = op.apply(None, _params(ref="refs/heads/main"), _env(history))
    feature = op.apply(None, _params(ref="refs/heads/feature"), _env(history))

    assert history["ref_heads"]["refs/heads/main"]["commit_id"] == main["commit_entry"]["id"]
    assert history["ref_heads"]["refs/heads/feature"]["commit_id"] == feature["commit_entry"]["id"]
    assert "heads" not in history
