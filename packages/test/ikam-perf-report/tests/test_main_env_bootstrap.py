from __future__ import annotations

import os
import subprocess
import sys
import ast
from pathlib import Path


def test_main_import_bootstraps_required_llm_env_from_local_dotenv() -> None:
    repo_root = Path(__file__).resolve().parents[4]
    pythonpath = os.pathsep.join(
        [
            str(repo_root / "packages" / "test" / "ikam-perf-report"),
            str(repo_root / "packages" / "modelado" / "src"),
            str(repo_root / "packages" / "ikam" / "src"),
        ]
    )
    env = {
        "PATH": os.environ.get("PATH", ""),
        "PYTHONPATH": pythonpath,
        "IKAM_CASES_ROOT": str(repo_root / "tests" / "fixtures" / "cases"),
    }
    result = subprocess.run(
        [sys.executable, "-c", "import ikam_perf_report.main"],
        cwd=str(repo_root),
        env=env,
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr


def test_main_module_does_not_require_python_dotenv_at_import_time() -> None:
    main_source = (Path(__file__).resolve().parents[1] / "ikam_perf_report" / "main.py").read_text(encoding="utf-8")
    tree = ast.parse(main_source)
    top_level_dotenv_import = False
    for node in tree.body:
        if isinstance(node, ast.ImportFrom) and node.module == "dotenv":
            top_level_dotenv_import = True
            break
    assert not top_level_dotenv_import
