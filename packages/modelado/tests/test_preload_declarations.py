from __future__ import annotations

from contextlib import contextmanager
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/modelado/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))

from modelado.graph.preload_declarations import load_executor_declarations
from modelado.plans import preload as preload_module
from modelado.plans.preload import (
    build_agent_spec_declaration_fragments,
    build_executor_declaration_fragments,
    build_workflow_compilation_fragments,
    preload_fixtures,
)


def test_load_executor_declarations_reads_consolidated_preseed_declarations() -> None:
    declarations = load_executor_declarations()

    by_id = {declaration.executor_id: declaration for declaration in declarations}

    assert set(by_id) == {"executor://python-primary", "executor://ml-primary", "executor://agent-env-primary"}
    assert by_id["executor://agent-env-primary"].executor_kind == "agent-executor"
    assert by_id["executor://python-primary"].executor_kind == "python-executor"
    assert by_id["executor://ml-primary"].executor_kind == "ml-executor"


def test_load_executor_declarations_rejects_invalid_yaml_payload(tmp_path: Path) -> None:
    bad_file = tmp_path / "bad.yaml"
    bad_file.write_text("[]\n", encoding="utf-8")

    try:
        load_executor_declarations(tmp_path)
    except ValueError as exc:
        assert str(exc) == "executor declaration file must decode to an object"
    else:
        raise AssertionError("expected invalid declaration payload to be rejected")


def test_build_executor_declaration_fragments_from_consolidated_preseed() -> None:
    fragments = build_executor_declaration_fragments()

    assert [fragment.data["executor_id"] for fragment in fragments] == [
        "executor://agent-env-primary",
        "executor://ml-primary",
        "executor://python-primary",
    ]


def test_build_workflow_compilation_fragments_include_source_and_lowered_graph() -> None:
    fragments = build_workflow_compilation_fragments()
    profiles = [getattr(fragment, "profile", None) for fragment in fragments]

    assert "rich_petri_workflow" in profiles
    assert "ikam_executable_graph" in profiles
    assert "ikam_graph_derivation" in profiles
    executable_graph = next(fragment for fragment in fragments if getattr(fragment, "profile", None) == "ikam_executable_graph")

    assert executable_graph.data["publish"] == [
        {
            "registry": "petri_net_runnables",
            "key": "ingestion-early-parse",
            "title": "Early Ingestion Parse",
            "goal": "Load documents, chunk them, and extract early semantic structure",
        }
    ]


def test_build_agent_spec_declaration_fragments_from_consolidated_preseed() -> None:
    fragments = build_agent_spec_declaration_fragments()

    assert len(fragments) == 1
    fragment = fragments[0]
    assert fragment.profile == "agent_spec"
    assert fragment.data["logical_name"] == "parse-review-agent"
    assert fragment.data["content"]["file"] == "support/agents/parse-review-agent.md"
    content_ref = fragment.data.get("content_ref")
    assert isinstance(content_ref, dict)
    assert content_ref["type"] == "cas_ref"
    assert content_ref["mime_type"] == "text/markdown"
    assert content_ref["role"] == "agent_summary"


def test_preload_fixtures_inserts_agent_spec_markdown_content(tmp_path: Path, monkeypatch) -> None:
    preseed_root = tmp_path / "preseed"
    declarations_dir = preseed_root / "declarations"
    support_agents_dir = preseed_root / "support" / "agents"
    declarations_dir.mkdir(parents=True)
    support_agents_dir.mkdir(parents=True)
    (preseed_root / "compiled").mkdir()

    (support_agents_dir / "parse-review-agent.md").write_text("# Parse Review Agent\n\nReviews semantic maps.\n", encoding="utf-8")
    (declarations_dir / "agent-executor.yaml").write_text((ROOT / "packages/test/ikam-perf-report/preseed/declarations/agent-executor.yaml").read_text(encoding="utf-8"), encoding="utf-8")
    (declarations_dir / "parse-review-agent.yaml").write_text((ROOT / "packages/test/ikam-perf-report/preseed/declarations/parse-review-agent.yaml").read_text(encoding="utf-8"), encoding="utf-8")

    inserted_rows: list[tuple[str, str, object, str]] = []
    inserted_ir: list[object] = []

    @contextmanager
    def fake_connection_scope():
        yield type("FakeConnection", (), {"commit": lambda self: None})()

    monkeypatch.setattr(preload_module, "connection_scope", fake_connection_scope)
    monkeypatch.setattr(preload_module, "build_workflow_compilation_fragments", lambda root=None: [])
    monkeypatch.setattr(preload_module, "get_shared_registry_manager", lambda: type("FakeRegistryManager", (), {"append_put": lambda self, cx, namespace, key, value, base_version=None: None})(), raising=False)
    monkeypatch.setattr(preload_module, "_insert_fragment_row", lambda cx, *, cas_id, mime_type, value, project_id: inserted_rows.append((cas_id, mime_type, value, project_id)))
    monkeypatch.setattr(preload_module, "_insert_ir_fragment", lambda cx, fragment, *, project_id: inserted_ir.append(fragment))

    preload_fixtures(preseed_root)

    assert any(mime_type == "text/markdown" for _, mime_type, _, _ in inserted_rows)
    assert any(getattr(fragment, "profile", None) == "agent_spec" for fragment in inserted_ir)


def test_preload_fixtures_publishes_agent_spec_registry_entry(tmp_path: Path, monkeypatch) -> None:
    preseed_root = tmp_path / "preseed"
    declarations_dir = preseed_root / "declarations"
    support_agents_dir = preseed_root / "support" / "agents"
    declarations_dir.mkdir(parents=True)
    support_agents_dir.mkdir(parents=True)
    (preseed_root / "compiled").mkdir()

    (support_agents_dir / "parse-review-agent.md").write_text("# Parse Review Agent\n", encoding="utf-8")
    (declarations_dir / "agent-executor.yaml").write_text((ROOT / "packages/test/ikam-perf-report/preseed/declarations/agent-executor.yaml").read_text(encoding="utf-8"), encoding="utf-8")
    (declarations_dir / "parse-review-agent.yaml").write_text((ROOT / "packages/test/ikam-perf-report/preseed/declarations/parse-review-agent.yaml").read_text(encoding="utf-8"), encoding="utf-8")

    publishes: list[tuple[str, str, object]] = []

    class FakeRegistryManager:
        def append_put(self, cx, namespace: str, key: str, value: object, *, base_version=None):
            publishes.append((namespace, key, value))

    @contextmanager
    def fake_connection_scope():
        yield type("FakeConnection", (), {"commit": lambda self: None})()

    monkeypatch.setattr(preload_module, "connection_scope", fake_connection_scope)
    monkeypatch.setattr(preload_module, "build_workflow_compilation_fragments", lambda root=None: [])
    monkeypatch.setattr(preload_module, "get_shared_registry_manager", lambda: FakeRegistryManager(), raising=False)
    monkeypatch.setattr(preload_module, "_insert_fragment_row", lambda *args, **kwargs: None)
    preload_fixtures(preseed_root)

    assert len(publishes) == 1
    namespace, key, value = publishes[0]
    assert namespace == "agent_specs"
    assert key == "parse-review-agent"
    assert isinstance(value, dict)
    assert value["type"] == "subgraph_ref"
    assert value["title"] == "Parse Review Agent"


def test_preload_fixtures_wires_generated_declarations_and_workflow_fragments(tmp_path: Path, monkeypatch) -> None:
    compiled_yaml = tmp_path / "compiled.yaml"
    compiled_yaml.write_text(
        """graph_id: test/project\nfragments:\n  - cas_id: cas://compiled\n    mime_type: application/json\n    value:\n      kind: compiled\n""",
        encoding="utf-8",
    )

    inserts: list[tuple[str, object, str]] = []
    commits: list[str] = []
    publishes: list[tuple[str, str, object]] = []

    class FakeConnection:
        def commit(self) -> None:
            commits.append("commit")

    class FakeRegistryManager:
        def append_put(self, cx, namespace: str, key: str, value: object, *, base_version=None):
            publishes.append((namespace, key, value))

    @contextmanager
    def fake_connection_scope():
        yield FakeConnection()

    monkeypatch.setattr(preload_module, "connection_scope", fake_connection_scope)
    monkeypatch.setattr(preload_module, "build_executor_declaration_fragments", lambda root=None: ["executor-fragment"])
    workflow_fragment = type(
        "FakeFragment",
        (),
        {
            "profile": "ikam_executable_graph",
            "cas_id": "cas://workflow-graph",
            "fragment_id": "graph:ingestion-early-parse:2026-03-21",
            "data": {
                "publish": [
                    {
                        "registry": "petri_net_runnables",
                        "key": "ingestion-early-parse",
                        "title": "Early Ingestion Parse",
                        "goal": "Load documents, chunk them, and extract early semantic structure",
                    }
                ]
            },
            "model_dump": lambda self, mode="json": {
                "profile": "ikam_executable_graph",
                "data": {
                    "publish": [
                        {
                            "registry": "petri_net_runnables",
                            "key": "ingestion-early-parse",
                            "title": "Early Ingestion Parse",
                            "goal": "Load documents, chunk them, and extract early semantic structure",
                        }
                    ]
                },
            },
        },
    )()
    monkeypatch.setattr(preload_module, "build_workflow_compilation_fragments", lambda root=None: [workflow_fragment])
    monkeypatch.setattr(
        preload_module,
        "_insert_ir_fragment",
        lambda cx, fragment, *, project_id: inserts.append(("ir", fragment, project_id)),
    )
    monkeypatch.setattr(
        preload_module,
        "_insert_fragment_row",
        lambda cx, *, cas_id, mime_type, value, project_id: inserts.append(("row", value, project_id)),
    )
    monkeypatch.setattr(preload_module, "get_shared_registry_manager", lambda: FakeRegistryManager())

    preload_fixtures(tmp_path)

    assert len(publishes) == 1
    published_value = publishes[0][2]
    assert isinstance(published_value, dict)

    assert inserts == [
        ("ir", "executor-fragment", "modelado/projects/canonical"),
        ("ir", workflow_fragment, "modelado/projects/canonical"),
        ("row", {"kind": "compiled"}, "test/project"),
    ]
    assert publishes == [
        (
            "petri_net_runnables",
            "ingestion-early-parse",
            {
                "type": "subgraph_ref",
                "fragment_id": "graph:ingestion-early-parse:2026-03-21",
                "head_fragment_id": preload_module._fragment_cas_id(workflow_fragment),
                "title": "Early Ingestion Parse",
                "goal": "Load documents, chunk them, and extract early semantic structure",
                "registered_at": published_value["registered_at"],
            },
        )
    ]
    assert commits == ["commit"]


def test_preload_fixtures_uses_passed_fixture_set_root_for_generated_fragments(tmp_path: Path, monkeypatch) -> None:
    custom_root = tmp_path / "custom-preseed"
    custom_root.mkdir()
    fixtures_dir = custom_root / "artifacts"
    fixtures_dir.mkdir()
    (fixtures_dir / "compiled.yaml").write_text("graph_id: test/project\nfragments: []\n", encoding="utf-8")

    called_roots: list[Path] = []

    @contextmanager
    def fake_connection_scope():
        yield type("FakeConnection", (), {"commit": lambda self: None})()

    monkeypatch.setattr(preload_module, "connection_scope", fake_connection_scope)
    def fake_build_executor_declaration_fragments(root: Path | None = None) -> list[object]:
        assert root is not None
        called_roots.append(root)
        return []

    def fake_build_workflow_compilation_fragments(root: Path | None = None) -> list[object]:
        assert root is not None
        called_roots.append(root)
        return []

    monkeypatch.setattr(preload_module, "build_executor_declaration_fragments", fake_build_executor_declaration_fragments)
    monkeypatch.setattr(preload_module, "build_workflow_compilation_fragments", fake_build_workflow_compilation_fragments)
    monkeypatch.setattr(preload_module, "_insert_fragment_row", lambda *args, **kwargs: None)

    preload_fixtures(fixtures_dir)

    assert called_roots == [custom_root, custom_root]


def test_preload_fixtures_loads_compiled_subdir_when_given_consolidated_root(tmp_path: Path, monkeypatch) -> None:
    consolidated_root = tmp_path / "preseed"
    consolidated_root.mkdir()
    compiled_dir = consolidated_root / "compiled"
    compiled_dir.mkdir()
    (compiled_dir / "compiled.yaml").write_text(
        """graph_id: test/project\nfragments:\n  - cas_id: cas://compiled\n    mime_type: application/json\n    value:\n      kind: compiled\n""",
        encoding="utf-8",
    )

    inserts: list[tuple[str, object, str]] = []

    @contextmanager
    def fake_connection_scope():
        yield type("FakeConnection", (), {"commit": lambda self: None})()

    monkeypatch.setattr(preload_module, "connection_scope", fake_connection_scope)
    monkeypatch.setattr(preload_module, "build_executor_declaration_fragments", lambda root=None: [])
    monkeypatch.setattr(preload_module, "build_workflow_compilation_fragments", lambda root=None: [])
    monkeypatch.setattr(
        preload_module,
        "_insert_fragment_row",
        lambda cx, *, cas_id, mime_type, value, project_id: inserts.append((cas_id, value, project_id)),
    )

    preload_fixtures(consolidated_root)

    assert inserts == [("cas://compiled", {"kind": "compiled"}, "test/project")]


def test_preload_fixtures_does_not_leak_previous_yaml_project_id(tmp_path: Path, monkeypatch) -> None:
    first_yaml = tmp_path / "01-first.yaml"
    first_yaml.write_text(
        """graph_id: first/project\nfragments:\n  - cas_id: cas://first\n    mime_type: application/json\n    value:\n      kind: first\n""",
        encoding="utf-8",
    )
    second_yaml = tmp_path / "02-second.yaml"
    second_yaml.write_text(
        """fragments:\n  - cas_id: cas://second\n    mime_type: application/json\n    value:\n      kind: second\n""",
        encoding="utf-8",
    )

    recorded_projects: list[str] = []

    @contextmanager
    def fake_connection_scope():
        yield type("FakeConnection", (), {"commit": lambda self: None})()

    monkeypatch.setattr(preload_module, "connection_scope", fake_connection_scope)
    monkeypatch.setattr(preload_module, "build_executor_declaration_fragments", lambda root=None: [])
    monkeypatch.setattr(preload_module, "build_workflow_compilation_fragments", lambda root=None: [])
    monkeypatch.setattr(
        preload_module,
        "_insert_fragment_row",
        lambda cx, *, cas_id, mime_type, value, project_id: recorded_projects.append(project_id),
    )

    preload_fixtures(tmp_path)

    assert recorded_projects == ["first/project", "modelado/projects/canonical"]
