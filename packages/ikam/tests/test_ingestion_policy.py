"""Tests for IngestionPolicy."""
from dataclasses import FrozenInstanceError
import pytest


def test_default_policy_does_not_retain_bytes():
    from ikam.forja.ingestion_policy import IngestionPolicy
    policy = IngestionPolicy()
    assert policy.retain_original_bytes is False


def test_opt_in_retains_bytes():
    from ikam.forja.ingestion_policy import IngestionPolicy
    policy = IngestionPolicy(retain_original_bytes=True)
    assert policy.retain_original_bytes is True


def test_frozen():
    from ikam.forja.ingestion_policy import IngestionPolicy
    policy = IngestionPolicy()
    with pytest.raises(FrozenInstanceError):
        policy.retain_original_bytes = True
