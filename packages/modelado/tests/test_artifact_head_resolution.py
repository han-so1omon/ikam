from __future__ import annotations

from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/modelado/src"))

import pytest  # noqa: E402

from modelado.environment_scope import EnvironmentScope  # noqa: E402


class _FakeCursorResult:
    def __init__(self, row):
        self._row = row

    def fetchone(self):
        return self._row


class _FakeBranchConnection:
    def __init__(self, rows: dict[tuple[str, str], dict[str, object] | None], commits: dict[str, dict[str, object] | None]) -> None:
        self.rows = rows
        self.commits = commits
        self.calls: list[tuple[str, tuple[object, ...]]] = []

    def execute(self, query: str, params: tuple[object, ...]):
        self.calls.append((query, params))
        if "FROM ikam_artifact_branches" in query:
            artifact_id, branch_name = params
            branch = self.rows.get((str(artifact_id), str(branch_name)))
            if branch is None:
                return _FakeCursorResult(None)
            commit = self.commits.get(str(branch.get("head_commit_id")))
            if commit is None:
                return _FakeCursorResult(branch)
            return _FakeCursorResult(
                {
                    "artifact_id": branch.get("artifact_id"),
                    "branch_name": branch.get("branch_name"),
                    "head_commit_id": branch.get("head_commit_id"),
                    "result_ref": commit.get("result_ref"),
                }
            )
        commit_id = params[0]
        commit = self.commits.get(str(commit_id))
        if commit is None:
            return _FakeCursorResult(None)
        return _FakeCursorResult(commit)


def test_resolve_artifact_head_returns_ref_scoped_head_object_id() -> None:
    from modelado.history.head_locators import resolve_artifact_head

    cx = _FakeBranchConnection(
        {
            ("artifact-semantic-1", "main"): {
                "artifact_id": "artifact-semantic-1",
                "branch_name": "main",
                "head_commit_id": "iac-main",
            }
        },
        {
            "iac-main": {
                "id": "iac-main",
                "artifact_id": "artifact-semantic-1",
                "branch_name": "main",
                "result_ref": {"ref": "refs/heads/main", "head_object_id": "obj-main-head"},
            }
        },
    )

    resolved = resolve_artifact_head(
        "ref://refs/heads/main/artifact/artifact-semantic-1",
        env_scope=EnvironmentScope(ref="refs/heads/feature/demo"),
        cx=cx,
    )

    assert resolved.semantic_id == "artifact-semantic-1"
    assert resolved.ref == "refs/heads/main"
    assert resolved.head_object_id == "obj-main-head"
    assert resolved.head_commit_id == "iac-main"
    assert "FROM ikam_artifact_branches" in cx.calls[0][0]


def test_artifact_shorthand_locator_fails_when_current_ref_has_no_head() -> None:
    from modelado.history.head_locators import resolve_artifact_head

    cx = _FakeBranchConnection({}, {})

    with pytest.raises(LookupError):
        resolve_artifact_head(
            "artifact://artifact-semantic-1",
            env_scope=EnvironmentScope(ref="refs/heads/feature/demo"),
            cx=cx,
        )


def test_resolve_artifact_head_uses_branch_head_commit_for_explicit_ref() -> None:
    from modelado.history.head_locators import resolve_ref_scoped_artifact_head

    cx = _FakeBranchConnection(
        {
            ("artifact-semantic-1", "main"): {
                "artifact_id": "artifact-semantic-1",
                "branch_name": "main",
                "head_commit_id": "iac-main",
            },
            ("artifact-semantic-1", "feature/demo"): {
                "artifact_id": "artifact-semantic-1",
                "branch_name": "feature/demo",
                "head_commit_id": "iac-feature",
            },
        },
        {
            "iac-main": {
                "id": "iac-main",
                "artifact_id": "artifact-semantic-1",
                "branch_name": "main",
                "result_ref": {"ref": "refs/heads/main", "head_object_id": "obj-main-head"},
            },
            "iac-feature": {
                "id": "iac-feature",
                "artifact_id": "artifact-semantic-1",
                "branch_name": "feature/demo",
                "result_ref": {"ref": "refs/heads/feature/demo", "head_object_id": "obj-feature-head"},
            },
        },
    )

    resolved = resolve_ref_scoped_artifact_head(
        "ref://refs/heads/main/artifact/artifact-semantic-1",
        env_scope=EnvironmentScope(ref="refs/heads/feature/demo"),
        cx=cx,
    )

    assert resolved.semantic_id == "artifact-semantic-1"
    assert resolved.ref == "refs/heads/main"
    assert resolved.head_object_id == "obj-main-head"
    assert resolved.head_commit_id == "iac-main"
    assert "FROM ikam_artifact_branches" in cx.calls[0][0]


def test_resolve_artifact_head_uses_current_env_ref_for_shorthand() -> None:
    from modelado.history.head_locators import resolve_ref_scoped_artifact_head

    cx = _FakeBranchConnection(
        {
            ("artifact-semantic-1", "feature/demo"): {
                "artifact_id": "artifact-semantic-1",
                "branch_name": "feature/demo",
                "head_commit_id": "iac-feature",
            }
        },
        {
            "iac-feature": {
                "id": "iac-feature",
                "artifact_id": "artifact-semantic-1",
                "branch_name": "feature/demo",
                "result_ref": {"ref": "refs/heads/feature/demo", "head_object_id": "obj-feature-head"},
            }
        },
    )

    resolved = resolve_ref_scoped_artifact_head(
        "artifact://artifact-semantic-1",
        env_scope=EnvironmentScope(ref="refs/heads/feature/demo"),
        cx=cx,
    )

    assert resolved.semantic_id == "artifact-semantic-1"
    assert resolved.ref == "refs/heads/feature/demo"
    assert resolved.head_object_id == "obj-feature-head"
    assert resolved.head_commit_id == "iac-feature"


def test_resolve_artifact_head_fails_when_ref_has_no_head() -> None:
    from modelado.history.head_locators import resolve_ref_scoped_artifact_head

    cx = _FakeBranchConnection({}, {})

    with pytest.raises(LookupError):
        resolve_ref_scoped_artifact_head(
            "ref://refs/heads/main/artifact/artifact-semantic-1",
            env_scope=EnvironmentScope(ref="refs/heads/feature/demo"),
            cx=cx,
        )


def test_resolve_artifact_head_scopes_commit_join_to_artifact() -> None:
    from modelado.history.head_locators import resolve_ref_scoped_artifact_head

    class _CrossArtifactConnection(_FakeBranchConnection):
        def execute(self, query: str, params: tuple[object, ...]):
            self.calls.append((query, params))
            if "FROM ikam_artifact_branches" in query:
                return _FakeCursorResult(
                    {
                        "artifact_id": "artifact-other",
                        "branch_name": "main",
                        "head_commit_id": "iac-other-artifact",
                        "result_ref": {"ref": "refs/heads/main", "head_object_id": "obj-other-artifact"},
                    }
                )
            return _FakeCursorResult(None)

    cx = _CrossArtifactConnection({}, {})

    with pytest.raises(LookupError):
        resolve_ref_scoped_artifact_head(
            "ref://refs/heads/main/artifact/artifact-semantic-1",
            env_scope=EnvironmentScope(ref="refs/heads/feature/demo"),
            cx=cx,
        )
