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


class _LineageConnection:
    def __init__(self) -> None:
        self.calls: list[tuple[str, tuple[object, ...]]] = []

    def execute(self, query: str, params: tuple[object, ...]):
        self.calls.append((query, params))
        if "FROM ikam_artifact_branches" in query:
            return _CursorResult(
                [
                    {
                        "artifact_id": "artifact-semantic-1",
                        "branch_name": "main",
                        "head_commit_id": "iac-main",
                        "result_ref": {"ref": "refs/heads/main", "head_object_id": "obj-main-head"},
                    }
                ]
            )
        if "FROM ikam_fragment_objects" in query:
            return _CursorResult([{"root_fragment_id": "frag-head-1"}])
        if "WHERE project_id = ?" in query and "out_id = ?" in query and params == ("project-1", "frag-head-1"):
            return _CursorResult([
                {"out_id": "frag-head-1", "in_id": "frag-child-1", "edge_label": "knowledge:derived_from", "t": 1}
            ])
        if "WHERE project_id = ?" in query and "in_id = ?" in query and params == ("project-1", "frag-head-1"):
            return _CursorResult([])
        return _CursorResult([])


def test_build_lineage_graph_uses_resolved_artifact_head_fragment_as_root() -> None:
    from modelado.knowledge_base.lineage import build_lineage_graph

    graph = build_lineage_graph(
        _LineageConnection(),
        artifact_id="artifact://artifact-semantic-1",
        project_id="project-1",
        env_scope=EnvironmentScope(ref="refs/heads/main"),
        depth=0,
    )

    assert graph["rootId"] == "frag-head-1"
    root = next(node for node in graph["nodes"] if node["id"] == graph["rootId"])
    assert root["type"] == "fragment"


def test_build_lineage_graph_accepts_direct_fragment_locator_as_root() -> None:
    from modelado.knowledge_base.lineage import build_lineage_graph

    graph = build_lineage_graph(
        _LineageConnection(),
        artifact_id="fragment://frag-previous-1",
        project_id="project-1",
        env_scope=EnvironmentScope(ref="refs/heads/main"),
        depth=0,
    )

    assert graph["rootId"] == "frag-previous-1"
    root = next(node for node in graph["nodes"] if node["id"] == graph["rootId"])
    assert root["type"] == "fragment"


def test_build_lineage_graph_keeps_raw_artifact_id_root_typed_as_artifact() -> None:
    from modelado.knowledge_base.lineage import build_lineage_graph

    graph = build_lineage_graph(
        _LineageConnection(),
        artifact_id="artifact-raw-1",
        project_id="project-1",
        depth=0,
    )

    root = next(node for node in graph["nodes"] if node["id"] == graph["rootId"])

    assert graph["rootId"] == "artifact-raw-1"
    assert root["type"] == "artifact"


def test_build_lineage_graph_types_non_root_fragment_nodes_when_traversing_fragments() -> None:
    from modelado.knowledge_base.lineage import build_lineage_graph

    graph = build_lineage_graph(
        _LineageConnection(),
        artifact_id="artifact://artifact-semantic-1",
        project_id="project-1",
        env_scope=EnvironmentScope(ref="refs/heads/main"),
        depth=1,
        direction="downstream",
    )

    child = next(node for node in graph["nodes"] if node["id"] == "frag-child-1")

    assert child["type"] == "fragment"


def test_build_lineage_graph_types_non_root_fragment_nodes_without_prefix_heuristic() -> None:
    from modelado.knowledge_base.lineage import build_lineage_graph

    class _NonPrefixedFragmentConnection(_LineageConnection):
        def execute(self, query: str, params: tuple[object, ...]):
            self.calls.append((query, params))
            if "FROM ikam_artifact_branches" in query:
                return _CursorResult(
                    [
                        {
                            "artifact_id": "artifact-semantic-1",
                            "branch_name": "main",
                            "head_commit_id": "iac-main",
                            "result_ref": {"ref": "refs/heads/main", "head_object_id": "obj-main-head"},
                        }
                    ]
                )
            if "FROM ikam_fragment_objects" in query:
                return _CursorResult([{"root_fragment_id": "fragment-child-root"}])
            if "WHERE project_id = ?" in query and "out_id = ?" in query and params == ("project-1", "fragment-child-root"):
                return _CursorResult(
                    [
                        {
                            "out_id": "fragment-child-root",
                            "in_id": "child-fragment-2",
                            "edge_label": "knowledge:derived_from",
                            "t": 1,
                        }
                    ]
                )
            if "WHERE project_id = ?" in query and "in_id = ?" in query and params == ("project-1", "fragment-child-root"):
                return _CursorResult([])
            return _CursorResult([])

    graph = build_lineage_graph(
        _NonPrefixedFragmentConnection(),
        artifact_id="artifact://artifact-semantic-1",
        project_id="project-1",
        env_scope=EnvironmentScope(ref="refs/heads/main"),
        depth=1,
        direction="downstream",
    )

    child = next(node for node in graph["nodes"] if node["id"] == "child-fragment-2")

    assert child["type"] == "fragment"
