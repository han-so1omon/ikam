
from __future__ import annotations
import pytest
pytest.importorskip('playwright')

import importlib.util
from pathlib import Path

import pytest


_SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "stagehand_perf_report.py"
_SPEC = importlib.util.spec_from_file_location("stagehand_perf_report", _SCRIPT)
if _SPEC is None or _SPEC.loader is None:
    raise RuntimeError(f"Unable to load stagehand script: {_SCRIPT}")
_MODULE = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_MODULE)


def test_env_scoped_drillthrough_requires_payload_reconciliation() -> None:
    checker = _MODULE.STAGEHAND_SCENARIOS["env_scoped_drillthrough"]["assert_contract"]
    with pytest.raises(AssertionError):
        checker(
            {
                "fragments": [{"id": "f1", "value": {"x": 1}, "meta": {"env_type": "dev"}}],
                "verification_records": [{"id": "v1"}],
                "reconstruction_programs": [{"id": "r1"}],
                "env_summary": {
                    "fragment_count": 1,
                    "verification_count": 2,
                    "reconstruction_program_count": 1,
                },
            }
        )


def test_evaluation_alignment_requires_debug_pipeline_block() -> None:
    checker = _MODULE.STAGEHAND_SCENARIOS["evaluation_alignment"]["assert_contract"]
    with pytest.raises(AssertionError):
        checker({"evaluation_payload": {"report": {"passed": True}, "details": {}}})


def test_cross_env_isolation_requires_isolated_mutation_and_delta_intent() -> None:
    checker = _MODULE.STAGEHAND_SCENARIOS["cross_env_isolation"]["assert_contract"]
    with pytest.raises(AssertionError):
        checker(
            {
                "scopes": {"dev": "dev-1", "staging": "stg-1", "committed": "main"},
                "write_delta": {"active_env": "dev", "mutated_envs": ["dev", "committed"]},
                "cross_env_read_copy": False,
                "delete_without_delta_error": True,
                "delete_with_delta_ok": True,
            }
        )
