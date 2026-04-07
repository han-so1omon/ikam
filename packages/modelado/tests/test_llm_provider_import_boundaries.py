from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1] / "src" / "modelado"


def _is_allowed_provider_file(path: Path) -> bool:
    normalized = str(path).replace("\\", "/")
    if "/oraculo/providers/" in normalized:
        return True
    if normalized.endswith("/oraculo/openai_embeddings.py"):
        return True
    if normalized.endswith("/oraculo/openai_judge.py"):
        return True
    return False


def test_direct_openai_imports_are_restricted_to_provider_adapters() -> None:
    violating_files: list[str] = []
    for file_path in ROOT.rglob("*.py"):
        if _is_allowed_provider_file(file_path):
            continue
        content = file_path.read_text(encoding="utf-8")
        if "from openai import" in content or "import openai" in content:
            violating_files.append(str(file_path.relative_to(ROOT.parent)))

    assert not violating_files, (
        "Direct OpenAI imports are only allowed in provider adapter modules. "
        f"Violations: {violating_files}"
    )
