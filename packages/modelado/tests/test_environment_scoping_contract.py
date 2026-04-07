from __future__ import annotations

import pytest

from modelado.environment_scope import EnvironmentScope, validate_cross_environment_mutation


def test_environment_scope_accepts_canonical_main_ref() -> None:
    scope = EnvironmentScope(ref="refs/heads/main")

    assert scope.ref == "refs/heads/main"


def test_environment_scope_accepts_canonical_run_ref() -> None:
    scope = EnvironmentScope(ref="refs/heads/run/run-123")

    assert scope.ref == "refs/heads/run/run-123"


def test_environment_scope_rejects_tier_only_inputs() -> None:
    with pytest.raises(TypeError):
        EnvironmentScope(env_type="committed", env_id="main")


def test_environment_scope_rejects_invalid_ref() -> None:
    with pytest.raises(ValueError, match="Invalid ref"):
        EnvironmentScope(ref="main")


def test_cross_environment_mutation_requires_delta_intents() -> None:
    target = EnvironmentScope(ref="refs/heads/run/dev-1")
    refs = [EnvironmentScope(ref="refs/heads/main")]
    with pytest.raises(ValueError):
        validate_cross_environment_mutation(target_scope=target, reference_scopes=refs, delta_intents=[])
