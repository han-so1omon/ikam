"""Contract tests for trace persistence schemas."""

import pytest
from pydantic import ValidationError

from interacciones.schemas import TracePersistenceMode, TracePersistencePolicy


def test_trace_persistence_policy_round_trip() -> None:
    policy = TracePersistencePolicy(mode=TracePersistenceMode.PER_STEP)

    assert policy.model_dump(mode="json") == {"mode": "per_step"}


def test_trace_persistence_policy_supports_batch_mode() -> None:
    policy = TracePersistencePolicy(mode=TracePersistenceMode.BATCH)

    assert policy.model_dump(mode="json") == {"mode": "batch"}


def test_trace_persistence_policy_rejects_invalid_mode() -> None:
    with pytest.raises(ValidationError):
        TracePersistencePolicy.model_validate({"mode": "always"})
