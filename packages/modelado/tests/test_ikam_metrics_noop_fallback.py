from __future__ import annotations

import importlib.util
import sys
import types
from pathlib import Path


def test_noop_metrics_fallback_exposes_required_interface() -> None:
    module_path = Path(__file__).resolve().parents[1] / "src" / "modelado" / "ikam_metrics.py"
    spec = importlib.util.spec_from_file_location("modelado_ikam_metrics_noop_test", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"unable to load module: {module_path}")

    previous = sys.modules.get("prometheus_client")
    sys.modules["prometheus_client"] = types.ModuleType("prometheus_client")
    try:
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
    finally:
        if previous is None:
            del sys.modules["prometheus_client"]
        else:
            sys.modules["prometheus_client"] = previous

    assert hasattr(module.ikam_info, "info")
    module.ikam_info.info({"version": "test"})
    module.record_cas_hit()
    module.record_cas_miss()
