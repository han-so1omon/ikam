from __future__ import annotations

import builtins
import importlib
from pathlib import Path
import sys

import pytest


ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/modelado/src"))
sys.path.insert(0, str(ROOT / "packages/ikam/src"))


def test_ikam_cli_imports_without_typer_for_graph_compile_helpers() -> None:
    original_import = builtins.__import__

    def blocked_import(name, globals=None, locals=None, fromlist=(), level=0):
        if name == "typer":
            raise ModuleNotFoundError("No module named 'typer'")
        return original_import(name, globals, locals, fromlist, level)

    sys.modules.pop("ikam.cli", None)
    sys.modules.pop("typer", None)
    builtins.__import__ = blocked_import

    try:
        cli = importlib.import_module("ikam.cli")
    finally:
        builtins.__import__ = original_import

    assert cli.PRESEED_OPERATORS_DIR == ROOT / "packages/test/ikam-perf-report/preseed/operators"
    assert callable(cli.compile_graph)


def test_ikam_cli_entrypoint_requires_typer_when_unavailable() -> None:
    original_import = builtins.__import__

    def blocked_import(name, globals=None, locals=None, fromlist=(), level=0):
        if name == "typer":
            raise ModuleNotFoundError("No module named 'typer'")
        return original_import(name, globals, locals, fromlist, level)

    sys.modules.pop("ikam.cli", None)
    sys.modules.pop("typer", None)
    builtins.__import__ = blocked_import

    try:
        cli = importlib.import_module("ikam.cli")
    finally:
        builtins.__import__ = original_import

    with pytest.raises(ModuleNotFoundError, match="typer is required"):
        cli.app()
