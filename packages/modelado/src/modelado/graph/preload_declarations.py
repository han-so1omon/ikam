from __future__ import annotations

from pathlib import Path

import yaml

from interacciones.schemas import ExecutorDeclaration
from modelado.preseed_paths import preseed_root


def load_executor_declarations(root: Path | None = None) -> list[ExecutorDeclaration]:
    base_root = root or preseed_root()
    declarations_root = base_root if base_root.name == "declarations" or list(base_root.glob("*.yaml")) else base_root / "declarations"
    declarations: list[ExecutorDeclaration] = []
    for yaml_file in sorted(declarations_root.glob("*.yaml")):
        data = yaml.safe_load(yaml_file.read_text(encoding="utf-8"))
        if not isinstance(data, dict):
            raise ValueError("executor declaration file must decode to an object")
        if data.get("profile") == "agent_spec":
            continue
        declarations.append(ExecutorDeclaration.model_validate(data))
    return declarations
