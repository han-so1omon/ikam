from __future__ import annotations

from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/modelado/src"))

from modelado.environment_scope import EnvironmentScope  # noqa: E402


class _CursorResult:
    def __init__(self, rows):
        self._rows = rows

    def fetchone(self):
        if not self._rows:
            return None
        return self._rows[0]

    def fetchall(self):
        return list(self._rows)


class _InferenceConnection:
    def __init__(self) -> None:
        self.calls: list[tuple[str, tuple[object, ...]]] = []

    def execute(self, query: str, params: tuple[object, ...]):
        self.calls.append((query, params))
        if "JOIN artifacts a ON a.id = af.artifact_id" in query:
            return _CursorResult([{"project_id": "project-1"}])
        if "SELECT artifact_id::text AS artifact_id" in query and "WHERE fragment_id = %s" in query:
            return _CursorResult([
                {"artifact_id": "artifact-a", "similarity_ratio": 1.0},
                {"artifact_id": "artifact-b", "similarity_ratio": 1.0},
            ])
        return _CursorResult([])


def test_suggest_fragment_completion_accepts_target_ref_locator() -> None:
    from modelado import ikam_inference

    cx = _InferenceConnection()

    result = ikam_inference.suggest_fragment_completion(
        cx,
        target_ref="fragment://frag-base-1",
        env_scope=EnvironmentScope(ref="refs/heads/main"),
        context_window=1,
    )

    assert result == []


def test_suggest_fragment_completion_returns_fragment_suggestions_for_fragment_target(monkeypatch) -> None:
    from modelado import ikam_inference

    def _fake_fetch_effective_derivation_edges(*_, **kwargs):
        node_ids = kwargs["node_ids"]
        if node_ids == ["frag-base-1"]:
            return [
                {
                    "out_id": "frag-parent-1",
                    "in_id": "frag-base-1",
                    "derivation_id": "drv-parent",
                    "derivation_type": "delta",
                    "parameters": {},
                }
            ]
        if node_ids == ["frag-parent-1"]:
            return [
                {
                    "out_id": "frag-parent-1",
                    "in_id": "frag-sibling-1",
                    "derivation_id": "drv-sibling",
                    "derivation_type": "delta",
                    "parameters": {},
                },
                {
                    "out_id": "frag-parent-1",
                    "in_id": "frag-sibling-2",
                    "derivation_id": "drv-sibling-2",
                    "derivation_type": "delta",
                    "parameters": {},
                }
            ]
        return []

    monkeypatch.setattr(ikam_inference, "_fetch_effective_derivation_edges", _fake_fetch_effective_derivation_edges)

    class _FragmentSuggestionConnection(_InferenceConnection):
        def execute(self, query: str, params: tuple[object, ...]):
            self.calls.append((query, params))
            if "JOIN artifacts a ON a.id = af.artifact_id" in query:
                return _CursorResult([{"project_id": "project-1"}])
            if "SELECT fragment_id," in query and "WHERE fragment_id = ANY(%s)" in query:
                return _CursorResult(
                    [
                        {
                            "fragment_id": "frag-suggested-1",
                            "reuse_count": 1,
                            "supporting_artifacts": ["frag-sibling-1"],
                        }
                    ]
                )
            return _CursorResult([])

    suggestions = ikam_inference.suggest_fragment_completion(
        _FragmentSuggestionConnection(),
        target_ref="fragment://frag-base-1",
        env_scope=EnvironmentScope(ref="refs/heads/main"),
        context_window=1,
        min_confidence=0.0,
    )

    assert len(suggestions) == 1
    assert suggestions[0].fragment_id == "frag-suggested-1"
    assert suggestions[0].provenance_support == ["frag-sibling-1"]
    assert suggestions[0].reasoning == "Found in 1 similar target(s) with shared provenance"


def test_suggest_fragment_completion_allows_fragment_endpoint_graph_edges(monkeypatch) -> None:
    from modelado import ikam_inference

    seen_node_ids: list[list[str]] = []

    def _fake_fetch_effective_derivation_edges(*_, **kwargs):
        node_ids = kwargs["node_ids"]
        seen_node_ids.append(list(node_ids))
        if node_ids == ["frag-base-1"]:
            return [
                {
                    "out_id": "frag-parent-1",
                    "in_id": "frag-base-1",
                    "derivation_id": "drv-parent",
                    "derivation_type": "delta",
                    "parameters": {},
                }
            ]
        if node_ids == ["frag-parent-1"]:
            return [
                {
                    "out_id": "frag-parent-1",
                    "in_id": "frag-sibling-1",
                    "derivation_id": "drv-sibling",
                    "derivation_type": "delta",
                    "parameters": {},
                }
            ]
        return []

    monkeypatch.setattr(ikam_inference, "_fetch_effective_derivation_edges", _fake_fetch_effective_derivation_edges)

    class _FragmentEndpointConnection(_InferenceConnection):
        def execute(self, query: str, params: tuple[object, ...]):
            self.calls.append((query, params))
            if "JOIN artifacts a ON a.id = af.artifact_id" in query:
                return _CursorResult([{"project_id": "project-1"}])
            if "SELECT fragment_id," in query and "WHERE fragment_id = ANY(%s)" in query:
                return _CursorResult(
                    [
                        {
                            "fragment_id": "frag-suggested-1",
                            "reuse_count": 1,
                            "supporting_artifacts": ["frag-sibling-1"],
                        }
                    ]
                )
            return _CursorResult([])

    suggestions = ikam_inference.suggest_fragment_completion(
        _FragmentEndpointConnection(),
        target_ref="fragment://frag-base-1",
        env_scope=EnvironmentScope(ref="refs/heads/main"),
        context_window=1,
        min_confidence=0.0,
    )

    assert ["frag-base-1"] in seen_node_ids
    assert ["frag-parent-1"] in seen_node_ids
    assert suggestions[0].provenance_support == ["frag-sibling-1"]


def test_suggest_fragment_completion_scores_fragment_target_support_from_real_sql_shape(monkeypatch) -> None:
    from modelado import ikam_inference

    def _fake_fetch_effective_derivation_edges(*_, **kwargs):
        node_ids = kwargs["node_ids"]
        if node_ids == ["frag-base-1"]:
            return [
                {
                    "out_id": "frag-parent-1",
                    "in_id": "frag-base-1",
                    "derivation_id": "drv-parent",
                    "derivation_type": "delta",
                    "parameters": {},
                }
            ]
        if node_ids == ["frag-parent-1"]:
            return [
                {
                    "out_id": "frag-parent-1",
                    "in_id": "frag-sibling-1",
                    "derivation_id": "drv-sibling",
                    "derivation_type": "delta",
                    "parameters": {},
                }
            ]
        return []

    monkeypatch.setattr(ikam_inference, "_fetch_effective_derivation_edges", _fake_fetch_effective_derivation_edges)

    class _RealSqlShapeConnection(_InferenceConnection):
        def execute(self, query: str, params: tuple[object, ...]):
            self.calls.append((query, params))
            if "JOIN artifacts a ON a.id = af.artifact_id" in query:
                return _CursorResult([{"project_id": "project-1"}])
            if "SELECT fragment_id," in query and "WHERE fragment_id = ANY(%s)" in query:
                return _CursorResult(
                    [
                        {
                            "fragment_id": "frag-suggested-1",
                            "reuse_count": 2,
                            "supporting_artifacts": ["artifact-a", "artifact-b"],
                        }
                    ]
                )
            if "SELECT fragment_id" in query and "WHERE artifact_id = %s::uuid" in query:
                artifact_id = params[0]
                if artifact_id == "artifact-a":
                    return _CursorResult([{ "fragment_id": "frag-sibling-1" }])
                if artifact_id == "artifact-b":
                    return _CursorResult([{ "fragment_id": "frag-sibling-2" }])
                return _CursorResult([])
            if "SELECT artifact_id::text AS artifact_id" in query and "WHERE fragment_id = %s" in query:
                return _CursorResult(
                    [
                        {"artifact_id": "artifact-a", "similarity_ratio": 1.0},
                        {"artifact_id": "artifact-b", "similarity_ratio": 1.0},
                    ]
                )
            return _CursorResult([])

    suggestions = ikam_inference.suggest_fragment_completion(
        _RealSqlShapeConnection(),
        target_ref="fragment://frag-base-1",
        env_scope=EnvironmentScope(ref="refs/heads/main"),
        context_window=1,
    )

    assert len(suggestions) == 1
    assert suggestions[0].confidence >= 0.5
    assert suggestions[0].provenance_support == ["frag-sibling-1", "frag-sibling-2"]


def test_suggest_variation_returns_target_ref_for_fragment_locator(monkeypatch) -> None:
    from modelado import ikam_inference

    def _fake_fetch_effective_derivation_edges(*_, **kwargs):
        node_ids = kwargs["node_ids"]
        if node_ids == ["artifact-a"]:
            return [
                {
                    "out_id": "artifact-a",
                    "in_id": "artifact-child-a",
                    "derivation_id": "drv-a",
                    "derivation_type": "delta",
                    "parameters": {"op": "rewrite", "tone": "formal"},
                }
            ]
        if node_ids == ["artifact-b"]:
            return [
                {
                    "out_id": "artifact-b",
                    "in_id": "artifact-child-b",
                    "derivation_id": "drv-b",
                    "derivation_type": "delta",
                    "parameters": {"op": "rewrite", "tone": "formal"},
                }
            ]
        return []

    monkeypatch.setattr(ikam_inference, "_fetch_effective_derivation_edges", _fake_fetch_effective_derivation_edges)

    suggestions = ikam_inference.suggest_variation(
        _InferenceConnection(),
        target_ref="fragment://frag-base-1",
        env_scope=EnvironmentScope(ref="refs/heads/main"),
    )

    assert len(suggestions) == 1
    assert suggestions[0].target_ref == "ref://refs/heads/main/fragment/frag-base-1"
    assert suggestions[0].suggested_delta == {"op": "rewrite", "tone": "formal"}
    assert suggestions[0].similar_derivations == ["drv-a", "drv-b"]
