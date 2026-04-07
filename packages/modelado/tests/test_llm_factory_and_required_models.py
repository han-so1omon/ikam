from __future__ import annotations

import pytest

from modelado.config.llm_config import ProviderLLMSettings
from modelado.oraculo.factory import load_required_llm_settings_from_env, create_ai_client_from_env


def test_provider_settings_use_defaults_if_missing(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("LLM_PROVIDER", raising=False)
    monkeypatch.delenv("LLM_MODEL", raising=False)
    monkeypatch.delenv("LLM_EMBED_MODEL", raising=False)
    monkeypatch.delenv("LLM_JUDGE_MODEL", raising=False)

    settings = ProviderLLMSettings.from_env()

    assert settings.provider == "openai"
    assert settings.model == "gpt-4o-mini"
    assert settings.embed_model == "text-embedding-3-small"
    assert settings.judge_model == "gpt-4o-mini"


def test_provider_settings_load_from_env(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("LLM_PROVIDER", "openai")
    monkeypatch.setenv("LLM_MODEL", "gpt-4o-mini")
    monkeypatch.setenv("LLM_EMBED_MODEL", "text-embedding-3-large")
    monkeypatch.setenv("LLM_JUDGE_MODEL", "gpt-4o-mini")

    settings = load_required_llm_settings_from_env()
    assert settings.provider == "openai"
    assert settings.model == "gpt-4o-mini"
    assert settings.embed_model == "text-embedding-3-large"
    assert settings.judge_model == "gpt-4o-mini"


def test_factory_rejects_unsupported_provider(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("LLM_PROVIDER", "unsupported")
    monkeypatch.setenv("LLM_MODEL", "model-a")
    monkeypatch.setenv("LLM_EMBED_MODEL", "model-b")
    monkeypatch.setenv("LLM_JUDGE_MODEL", "model-c")

    with pytest.raises(ValueError) as exc_info:
        create_ai_client_from_env()

    assert "Unsupported AI provider" in str(exc_info.value)
