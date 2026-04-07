from __future__ import annotations

import pytest

from modelado.environment_scope import EnvironmentScope
from modelado.operators.commit import CommitOperator
from modelado.operators.core import OperatorEnv, OperatorParams


_DEV_SCOPE = EnvironmentScope(ref="refs/heads/run/run/test")


def _env() -> OperatorEnv:
    return OperatorEnv(seed=7, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)


def _sample_params(policy: str, *, include_evidence: bool = False, mapping_mode: str = "semantic_relations_only") -> OperatorParams:
    return OperatorParams(
        name="commit",
        parameters={
            "commit_policy": policy,
            "mapping_mode": mapping_mode,
            "include_evidence_surface_fragments": include_evidence,
            "target_ref": "refs/heads/main",
            "promoted_fragment_ids": ["prop-1", "semantic-subgraph-1"],
            "proposition_fragments": [{"id": "prop-1"}, {"id": "prop-2"}],
            "semantic_subgraph_snapshot": {"id": "semantic-subgraph-1"},
            "community_reports": [{"id": "report-1"}],
            "surface_fragments": [{"id": "surface-1"}],
            "reconstruction_artifacts": [{"id": "recon-1"}],
        },
    )


def test_semantic_relations_only_commit_list_excludes_surface_fragments() -> None:
    result = CommitOperator().apply(None, _sample_params("semantic_relations_only"), _env())
    ids = [entry["id"] for entry in result["commit_list"]]

    assert "prop-1" in ids
    assert "prop-2" in ids
    assert "semantic-subgraph-1" in ids
    assert "report-1" in ids
    assert "surface-1" not in ids


def test_semantic_relations_plus_evidence_includes_surface_fragments() -> None:
    result = CommitOperator().apply(
        None,
        _sample_params("semantic_relations_plus_evidence", include_evidence=True),
        _env(),
    )
    ids = [entry["id"] for entry in result["commit_list"]]
    assert "surface-1" in ids


def test_semantic_relations_plus_evidence_requires_flag() -> None:
    with pytest.raises(ValueError, match="include_evidence_surface_fragments"):
        CommitOperator().apply(None, _sample_params("semantic_relations_plus_evidence"), _env())


def test_semantic_relations_only_rejects_evidence_flag() -> None:
    with pytest.raises(ValueError, match="semantic_relations_plus_evidence"):
        CommitOperator().apply(
            None,
            _sample_params("semantic_relations_only", include_evidence=True),
            _env(),
        )


def test_full_preservation_policy_rejects_semantic_only_mode() -> None:
    with pytest.raises(ValueError, match="Invalid mapping_mode"):
        CommitOperator().apply(
            None,
            _sample_params("full_preservation", mapping_mode="semantic_relations_only"),
            _env(),
        )


def test_commit_result_exposes_target_ref_and_selected_promoted_fragment_ids() -> None:
    result = CommitOperator().apply(None, _sample_params("semantic_relations_only"), _env())

    assert result["target_ref"] == "refs/heads/main"
    assert result["promoted_fragment_ids"] == ["prop-1", "semantic-subgraph-1"]
