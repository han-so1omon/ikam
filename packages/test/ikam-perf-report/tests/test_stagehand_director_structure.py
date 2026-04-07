
from __future__ import annotations
import pytest
pytest.importorskip('playwright')

import importlib.util
from pathlib import Path


_SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "stagehand_perf_report.py"
_SPEC = importlib.util.spec_from_file_location("stagehand_perf_report", _SCRIPT)
if _SPEC is None or _SPEC.loader is None:
    raise RuntimeError(f"Unable to load stagehand script: {_SCRIPT}")
_MODULE = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_MODULE)
STAGEHAND_SCENARIOS = _MODULE.STAGEHAND_SCENARIOS


def test_stagehand_scenarios_cover_required_set() -> None:
    assert set(STAGEHAND_SCENARIOS) == {
        "core_run_stream",
        "controls_modes",
        "deterministic_retry",
        "env_scoped_drillthrough",
        "evaluation_alignment",
        "cross_env_isolation",
        "embedding_model_identity",
    }


def test_each_scenario_has_required_hooks() -> None:
    required = {"seed", "act", "assert_contract", "agentic_checkpoints", "collect_artifacts"}
    for scenario in STAGEHAND_SCENARIOS.values():
        assert required.issubset(set(scenario))
        for key in required:
            assert callable(scenario[key])


def test_core_run_stream_uses_real_seed_and_act_hooks() -> None:
    scenario = STAGEHAND_SCENARIOS["core_run_stream"]
    assert scenario["seed"] is _MODULE._scenario_seed_core_run_stream
    assert scenario["act"] is _MODULE._scenario_act_core_run_stream


def test_controls_modes_uses_real_run_button_stepthrough_hooks() -> None:
    scenario = STAGEHAND_SCENARIOS["controls_modes"]
    assert scenario["seed"] is _MODULE._scenario_seed_controls_modes
    assert scenario["act"] is _MODULE._scenario_act_controls_modes
