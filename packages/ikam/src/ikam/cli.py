import json
import re
import uuid
from collections.abc import Iterable
from pathlib import Path
from typing import Any, Dict, List, cast

import yaml
import blake3

try:
    import typer
except ModuleNotFoundError:  # pragma: no cover - exercised from outer package tests
    typer = None

from modelado.preseed_paths import preseed_compiled_dir, preseed_root, preseed_source_dirs

def _missing_typer_app() -> None:
    raise ModuleNotFoundError("typer is required to run the IKAM CLI")


if typer is None:
    app = _missing_typer_app
else:
    app = typer.Typer(help="IKAM Control Plane CLI", no_args_is_help=True)

IKAM_NAMESPACE = uuid.UUID("34927f8d-dbfa-4927-b5ba-1481bd2e35e7")
PRESEED_ROOT = preseed_root()
PRESEED_SOURCE_DIRS = preseed_source_dirs()
PRESEED_OPERATORS_DIR = PRESEED_ROOT / "operators"
PRESEED_COMPILED_DIR = preseed_compiled_dir()

def _canonicalize_json(payload: Any) -> bytes:
    """Serialize a payload to deterministic UTF-8 JSON bytes."""
    stable_json = json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return stable_json.encode("utf-8")

def _compute_cas_id(payload_bytes: bytes) -> str:
    """Return the cas_id (blake3 hash) for the canonical bytes."""
    return blake3.blake3(payload_bytes).hexdigest()

def _compute_fragment_id(logical_name: str) -> str:
    """Return a stable fragment_id (UUIDv5) based on the logical name."""
    return str(uuid.uuid5(IKAM_NAMESPACE, logical_name))

def _resolve_refs(data: Any, known_names: set[str]) -> Any:
    """Recursively resolve ${ref:id} strings dynamically using UUIDv5."""
    if isinstance(data, dict):
        return {k: _resolve_refs(v, known_names) for k, v in data.items()}
    elif isinstance(data, list):
        return [_resolve_refs(item, known_names) for item in data]
    elif isinstance(data, str):
        # Look for exactly ${ref:something}
        match = re.fullmatch(r"\$\{ref:([^}]+)\}", data)
        if match:
            ref_name = match.group(1)
            if ref_name not in known_names:
                raise ValueError(f"Unknown ref target: {ref_name}")
            return _compute_fragment_id(ref_name)
        # Look for embedded ${ref:something} inside strings
        def replace_match(m):
            ref_name = m.group(1)
            if ref_name not in known_names:
                raise ValueError(f"Unknown ref target: {ref_name}")
            return _compute_fragment_id(ref_name)
        return re.sub(r"\$\{ref:([^}]+)\}", replace_match, data)
    else:
        return data

def _collect_fragment_names(yaml_paths: Iterable[Path]) -> set[str]:
    known_names: set[str] = set()
    for yaml_path in yaml_paths:
        with open(yaml_path, "r", encoding="utf-8") as f:
            doc = yaml.safe_load(f) or {}
        for idx, fragment in enumerate(doc.get("fragments", [])):
            name = fragment.get("name")
            if not name:
                raise ValueError(f"Fragment at index {idx} in {yaml_path.name} is missing a logical 'name'.")
            if name in known_names:
                raise ValueError(f"Duplicate logical name found across preseed sources: {name}")
            known_names.add(name)
    return known_names


def compile_graph(yaml_path: Path, *, known_names: set[str] | None = None) -> Dict[str, Any]:
    with open(yaml_path, "r", encoding="utf-8") as f:
        doc = yaml.safe_load(f)

    if not doc or "fragments" not in doc:
        return doc
        
    fragments = doc["fragments"]
    known_names = known_names or {fragment.get("name") for fragment in fragments if fragment.get("name")}
    
    resolved_fragments: List[Dict[str, Any]] = []
    seen_names = set()
    
    for idx, f in enumerate(fragments):
        name = f.get("name")
        if not name:
            raise ValueError(f"Fragment at index {idx} in {yaml_path.name} is missing a logical 'name'.")
        if name in seen_names:
            raise ValueError(f"Duplicate logical name found in {yaml_path.name}: {name}")
        seen_names.add(name)
        
        resolved_value = _resolve_refs(f["value"], known_names)

        mime_type = f.get("mime_type", "application/octet-stream")
        if mime_type.startswith("application/vnd.ikam.ir."):
            if "ir_profile" not in resolved_value or not isinstance(resolved_value["ir_profile"], str):
                raise ValueError(f"Fragment {name} is missing a strictly singular 'ir_profile' string field.")
            if resolved_value["ir_profile"] == "RawBytesIR":
                raise ValueError(f"Fragment {name} uses 'RawBytesIR' which is forbidden in graph compilation. Use 'StructuredDataIR'.")

        if isinstance(resolved_value, dict):
            resolved_value = {**resolved_value, "_fragment_id": _compute_fragment_id(name)}

        payload_bytes = _canonicalize_json(resolved_value)
        cas_id = _compute_cas_id(payload_bytes)
        fragment_id = _compute_fragment_id(name)
        
        resolved_fragments.append({
            "name": name,
            "fragment_id": fragment_id,
            "cas_id": cas_id,
            "mime_type": mime_type,
            "value": resolved_value
        })
            
    doc["fragments"] = resolved_fragments
    return doc

def _compile_fixtures_impl(templates_dir: Path, output_dir: Path) -> None:
    """Compile YAML template graphs into mathematically sound YAML fixtures."""
    if not templates_dir.exists():
        raise FileNotFoundError(f"Templates directory {templates_dir} does not exist.")

    output_dir.mkdir(parents=True, exist_ok=True)

    operators_dir = templates_dir if templates_dir.name == "operators" or list(templates_dir.glob("*.yaml")) else templates_dir / "operators"
    source_dirs = [operators_dir]
    yaml_files = [yaml_file for source_dir in source_dirs for yaml_file in source_dir.glob("*.yaml") if source_dir.exists()]
    if not yaml_files:
        raise FileNotFoundError(f"No YAML preseed sources found under {templates_dir}.")

    known_names = _collect_fragment_names(yaml_files)

    for yaml_file in yaml_files:
        try:
            compiled_graph = compile_graph(yaml_file, known_names=known_names)
            out_file = output_dir / f"{yaml_file.stem}.yaml"
            with open(out_file, "w", encoding="utf-8") as f:
                yaml.dump(compiled_graph, f, default_flow_style=False, sort_keys=False)
        except Exception as exc:
            raise RuntimeError(f"Error compiling {yaml_file.name}: {exc}") from exc


def compile_fixtures(templates_dir: Path = PRESEED_ROOT, output_dir: Path = PRESEED_COMPILED_DIR) -> None:
    _compile_fixtures_impl(templates_dir, output_dir)


def version() -> str:
    return "0.1.0"


if typer is not None:
    cli_typer = typer
    cli_app = cast(Any, app)

    @cli_app.command(name="compile-fixtures")
    def compile_fixtures_command(
        templates_dir: Path = typer.Option(
            PRESEED_ROOT,
            help="Consolidated preseed root containing YAML sources"
        ),
        output_dir: Path = typer.Option(
            PRESEED_COMPILED_DIR,
            help="Directory to write compiled YAML graphs"
        )
    ) -> None:
        try:
            cli_typer.echo(f"Compiling fixtures from {templates_dir}...")
            compile_fixtures(templates_dir, output_dir)
        except FileNotFoundError as exc:
            cli_typer.echo(f"Error: {exc}")
            raise cli_typer.Exit(1) from exc
        except RuntimeError as exc:
            cli_typer.echo(str(exc))
            raise cli_typer.Exit(1) from exc


    @cli_app.command(name="version")
    def version_command() -> None:
        """Show version."""
        cli_typer.echo(version())

if __name__ == "__main__":
    app()
