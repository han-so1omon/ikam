
from __future__ import annotations
import pytest
pytest.importorskip('playwright')

import importlib.util
from pathlib import Path

import pytest

from modelado.fragment_embedder import get_shared_embedder


_SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "stagehand_perf_report.py"
_SPEC = importlib.util.spec_from_file_location("stagehand_perf_report", _SCRIPT)
if _SPEC is None or _SPEC.loader is None:
    raise RuntimeError(f"Unable to load stagehand script: {_SCRIPT}")
_MODULE = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_MODULE)


def test_core_stream_contract_rejects_non_canonical_order() -> None:
    checker = _MODULE.STAGEHAND_SCENARIOS["core_run_stream"]["assert_contract"]
    with pytest.raises(AssertionError):
        checker(
            {
                "events": [
                    {"attempt_index": 1, "step_name": "prepare_case", "step_id": "s1"},
                    {"attempt_index": 1, "step_name": "lift", "step_id": "s2"},
                ]
            }
        )


def test_controls_contract_requires_duplicate_idempotency_evidence() -> None:
    checker = _MODULE.STAGEHAND_SCENARIOS["controls_modes"]["assert_contract"]
    with pytest.raises(AssertionError):
        checker(
            {
                "control_responses": [
                    {"action": "set_mode", "status": "ok"},
                    {"action": "pause", "status": "ok"},
                    {"action": "resume", "status": "ok"},
                    {"action": "next_step", "status": "ok"},
                ],
                "before_events": [{"step_name": "lift"}],
                "after_events": [{"step_name": "lift"}, {"step_name": "embed_lifted"}],
            }
        )


def test_retry_contract_requires_retry_parent_linkage_and_increment() -> None:
    checker = _MODULE.STAGEHAND_SCENARIOS["deterministic_retry"]["assert_contract"]
    with pytest.raises(AssertionError):
        checker(
            {
                "events": [
                    {
                        "step_id": "verify-1",
                        "step_name": "verify",
                        "status": "failed",
                        "attempt_index": 1,
                        "retry_parent_step_id": None,
                    },
                    {
                        "step_id": "decompose-1",
                        "step_name": "map",
                        "status": "succeeded",
                        "attempt_index": 1,
                        "retry_parent_step_id": "verify-1",
                    },
                ]
            }
        )


def test_embedding_model_identity_contract_rejects_wrong_model() -> None:
    """_assert_embedding_model_identity_contract raises when model name is wrong."""
    checker = _MODULE.STAGEHAND_SCENARIOS["embedding_model_identity"]["assert_contract"]
    with pytest.raises(AssertionError):
        checker({"embedding_info": {"embedding_model": "wrong-model/v0", "vector_dims": 768, "fragment_count": 3}})


def test_embedding_model_identity_contract_rejects_wrong_dims() -> None:
    """_assert_embedding_model_identity_contract raises when vector dims are wrong."""
    checker = _MODULE.STAGEHAND_SCENARIOS["embedding_model_identity"]["assert_contract"]
    correct_model = get_shared_embedder().model_name
    with pytest.raises(AssertionError):
        checker({"embedding_info": {"embedding_model": correct_model, "vector_dims": 42, "fragment_count": 3}})


def test_embedding_model_identity_contract_rejects_db_unavailable() -> None:
    """_assert_embedding_model_identity_contract raises when db_unavailable."""
    checker = _MODULE.STAGEHAND_SCENARIOS["embedding_model_identity"]["assert_contract"]
    with pytest.raises(AssertionError):
        checker({"embedding_info": {"status": "db_unavailable"}})
