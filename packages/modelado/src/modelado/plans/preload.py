import json
import datetime as dt
import importlib
import importlib.util
import sys
from contextlib import nullcontext
from hashlib import blake2b
import yaml
from pathlib import Path
from typing import Any

from modelado.core.execution_context import ExecutionContext, ExecutionMode, execution_context, get_execution_context
from modelado.db import connection_scope
from modelado.preseed_paths import preseed_compiled_dir, preseed_root
from modelado.registry import get_shared_registry_manager


def default_preseed_root() -> Path:
    return preseed_root()


def default_compiled_preseed_dir() -> Path:
    return preseed_compiled_dir()


def _packages_root() -> Path:
    return Path(__file__).resolve().parents[4]


def _ensure_src_path(package_name: str) -> None:
    src_path = _packages_root() / package_name / "src"
    if str(src_path) not in sys.path:
        sys.path.insert(0, str(src_path))


def _load_workflow_preload_module() -> Any:
    module_name = "interacciones.workflow.preload_workflows"
    try:
        importlib.invalidate_caches()
        return importlib.import_module(module_name)
    except ModuleNotFoundError:
        module_path = _packages_root() / "interacciones" / "workflow" / "src" / "interacciones" / "workflow" / "preload_workflows.py"
        spec = importlib.util.spec_from_file_location(module_name, module_path)
        if spec is None or spec.loader is None:
            raise ImportError(f"unable to load workflow preload module from {module_path}")
        module = importlib.util.module_from_spec(spec)
        sys.modules[module_name] = module
        spec.loader.exec_module(module)
        return module


def build_executor_declaration_fragments(root: Path | None = None) -> list[Any]:
    _ensure_src_path("ikam")
    from ikam.ir.core import StructuredDataIR
    from modelado.graph.preload_declarations import load_executor_declarations

    declarations = load_executor_declarations(root)
    return [
        StructuredDataIR(
            artifact_id=f"artifact:executor-declaration:{declaration.executor_id}",
            fragment_id=f"executor-declaration:{declaration.executor_id}",
            profile="executor_declaration",
            data=declaration.model_dump(mode="json"),
        )
        for declaration in declarations
    ]


def _load_agent_spec_declarations(root: Path | None = None) -> list[dict[str, Any]]:
    base_root = root or default_preseed_root()
    declarations_root = base_root if base_root.name == "declarations" or list(base_root.glob("*.yaml")) else base_root / "declarations"
    declarations: list[dict[str, Any]] = []
    for yaml_file in sorted(declarations_root.glob("*.yaml")):
        data = yaml.safe_load(yaml_file.read_text(encoding="utf-8"))
        if isinstance(data, dict) and data.get("profile") == "agent_spec":
            declarations.append(data)
    return declarations


def _content_cas_id(payload: bytes) -> str:
    return f"cas://{blake2b(payload).hexdigest()}"


def build_agent_spec_declaration_fragments(root: Path | None = None) -> list[Any]:
    _ensure_src_path("ikam")
    from ikam.ir.core import StructuredDataIR

    base_root = root or default_preseed_root()
    fragments: list[Any] = []
    for declaration in _load_agent_spec_declarations(base_root):
        raw_content = declaration.get("content")
        if not isinstance(raw_content, dict):
            raise ValueError("agent spec content section is required")
        content = raw_content
        content_file = str(content.get("file") or "").strip()
        if not content_file:
            raise ValueError("agent spec content.file is required")
        content_bytes = (base_root / content_file).read_bytes()
        fragment_data = dict(declaration)
        fragment_data["content_ref"] = {
            "type": "cas_ref",
            "cas_id": _content_cas_id(content_bytes),
            "mime_type": str(content.get("mime_type") or "text/markdown"),
            "source_file": content_file,
            "role": str(content.get("content_role") or "agent_summary"),
        }
        logical_name = str(declaration.get("logical_name") or "agent-spec")
        fragments.append(
            StructuredDataIR(
                artifact_id=f"artifact:agent-spec:{logical_name}",
                fragment_id=f"agent-spec:{logical_name}",
                profile="agent_spec",
                data=fragment_data,
            )
        )
    return fragments


def build_workflow_compilation_fragments(root: Path | None = None) -> list[Any]:
    _ensure_src_path("ikam")
    _ensure_src_path("interacciones/workflow")
    from modelado.graph.compiler import GraphCompiler
    from modelado.graph.preload_declarations import load_executor_declarations

    load_compiled_workflows = _load_workflow_preload_module().load_compiled_workflows
    declarations = load_executor_declarations(root)
    compiler = GraphCompiler()
    fragments: list[Any] = []
    for workflow in load_compiled_workflows(root):
        fragments.extend(compiler.compile(workflow, executor_declarations=declarations).fragments)
    return fragments


def _source_root_for(fixtures_dir: Path) -> Path:
    if fixtures_dir.name == "compiled":
        return fixtures_dir.parent
    if any(fixtures_dir.glob("*.yaml")):
        return fixtures_dir.parent
    if (fixtures_dir / "compiled").is_dir():
        return fixtures_dir
    return default_preseed_root()


def _compiled_yaml_files(fixtures_dir: Path) -> list[Path]:
    if fixtures_dir.name == "compiled":
        return sorted(fixtures_dir.glob("*.yaml"))
    direct_yaml = sorted(fixtures_dir.glob("*.yaml"))
    if direct_yaml:
        return direct_yaml
    compiled_dir = fixtures_dir / "compiled"
    if compiled_dir.is_dir():
        return sorted(compiled_dir.glob("*.yaml"))
    return []


def _insert_fragment_row(cx: Any, *, cas_id: str, mime_type: str, value: Any, project_id: str) -> None:
    with cx.cursor() as cur:
        cur.execute(
            """
            INSERT INTO ikam_fragment_store (cas_id, mime_type, value, project_id, env)
            VALUES (%s, %s, %s, %s, %s)
            ON CONFLICT (cas_id, env, COALESCE(operation_id, '')) DO UPDATE
            SET value = EXCLUDED.value
            """,
            (
                cas_id,
                mime_type,
                json.dumps(value),
                project_id,
                "committed",
            ),
        )


def _insert_ir_fragment(cx: Any, fragment: Any, *, project_id: str) -> None:
    _ensure_src_path("ikam")
    from ikam.graph import _cas_hex

    value = fragment.model_dump(mode="json")
    cas_id = fragment.cas_id or _cas_hex(json.dumps(value, sort_keys=True).encode("utf-8"))
    mime_type = f"application/ikam-ir+{fragment.__class__.__name__.lower()}+json"
    _insert_fragment_row(cx, cas_id=cas_id, mime_type=mime_type, value=value, project_id=project_id)


def _fragment_cas_id(fragment: Any) -> str | None:
    _ensure_src_path("ikam")
    from ikam.graph import _cas_hex

    cas_id = getattr(fragment, "cas_id", None)
    if isinstance(cas_id, str) and cas_id:
        return cas_id
    model_dump = getattr(fragment, "model_dump", None)
    if callable(model_dump):
        return _cas_hex(json.dumps(model_dump(mode="json"), sort_keys=True).encode("utf-8"))
    return None


def _publish_declared_registries(cx: Any, fragment: Any) -> None:
    data = getattr(fragment, "data", None)
    if not isinstance(data, dict):
        return
    publish_targets = data.get("publish")
    if not isinstance(publish_targets, list):
        return
    head_fragment_id = _fragment_cas_id(fragment)
    fragment_id = getattr(fragment, "fragment_id", None)
    if not isinstance(head_fragment_id, str) or not isinstance(fragment_id, str):
        return
    manager = get_shared_registry_manager()
    publish_context = nullcontext()
    if get_execution_context() is None:
        publish_context = execution_context(
            ExecutionContext(
                mode=ExecutionMode.REQUEST,
                request_id="preload-fixtures",
                actor_id="preload",
                purpose="preload registry publication",
            )
        )
    with publish_context:
        for target in publish_targets:
            if not isinstance(target, dict):
                continue
            registry = target.get("registry")
            key = target.get("key")
            if not registry or not key:
                continue
            entry = {
                "type": "subgraph_ref",
                "fragment_id": fragment_id,
                "head_fragment_id": head_fragment_id,
                "registered_at": dt.datetime.now(dt.timezone.utc).isoformat(),
            }
            title = target.get("title")
            goal = target.get("goal")
            if title:
                entry["title"] = title
            if goal:
                entry["goal"] = goal
            manager.append_put(cx, namespace=registry, key=key, value=entry)

def preload_fixtures(fixtures_dir: Path) -> None:
    """
    Reads compiled YAML fixtures and inserts them into the ikam_fragment_store.
    This replaces hardcoded dicts with a truly dynamic graph-backed registry.
    """
    with connection_scope() as cx:
        source_root = _source_root_for(fixtures_dir)
        default_project_id = "modelado/projects/canonical"
        for fragment in build_executor_declaration_fragments(source_root):
            _insert_ir_fragment(cx, fragment, project_id=default_project_id)
        for declaration in _load_agent_spec_declarations(source_root):
            raw_content = declaration.get("content")
            if not isinstance(raw_content, dict):
                continue
            content = raw_content
            content_file = str(content.get("file") or "").strip()
            if not content_file:
                continue
            markdown = (source_root / content_file).read_text(encoding="utf-8")
            _insert_fragment_row(
                cx,
                cas_id=_content_cas_id(markdown.encode("utf-8")),
                mime_type=str(content.get("mime_type") or "text/markdown"),
                value={"text": markdown},
                project_id=default_project_id,
            )
        for fragment in build_agent_spec_declaration_fragments(source_root):
            _insert_ir_fragment(cx, fragment, project_id=default_project_id)
            _publish_declared_registries(cx, fragment)
        for fragment in build_workflow_compilation_fragments(source_root):
            _insert_ir_fragment(cx, fragment, project_id=default_project_id)
            _publish_declared_registries(cx, fragment)
        for yaml_file in _compiled_yaml_files(fixtures_dir):
            with open(yaml_file, "r") as f:
                doc = yaml.safe_load(f)

            project_id = doc.get("graph_id", default_project_id)
            fragments = doc.get("fragments", [])

            for frag_data in fragments:
                # Expecting the compiler to have generated cas_id, mime_type, and value
                cas_id = frag_data.get("cas_id")
                fragment_id = frag_data.get("fragment_id")
                mime_type = frag_data.get("mime_type", "application/json")
                value = frag_data.get("value")

                if not cas_id or value is None:
                    continue

                _insert_fragment_row(cx, cas_id=cas_id, mime_type=mime_type, value=value, project_id=project_id)

        cx.commit()

if __name__ == "__main__":
    import sys
    d = Path(sys.argv[1]) if len(sys.argv) > 1 else default_compiled_preseed_dir()
    preload_fixtures(d)
    print(f"Preloaded fixtures from {d}")
