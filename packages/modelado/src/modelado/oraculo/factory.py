"""Factory for provider-agnostic AI client instances."""

from __future__ import annotations

from modelado.config.llm_config import ProviderLLMSettings
from modelado.oraculo.ai_client import AIClient


def load_required_llm_settings_from_env() -> ProviderLLMSettings:
    """Load provider and required model knobs from environment."""
    return ProviderLLMSettings.from_env()


def create_ai_client_from_env() -> AIClient:
    """Create a unified AI client from required environment settings."""
    settings = load_required_llm_settings_from_env()
    if settings.provider == "openai":
        from modelado.oraculo.providers.openai_client import OpenAIUnifiedAIClient

        return OpenAIUnifiedAIClient(
            model=settings.model,
            embed_model=settings.embed_model,
            judge_model=settings.judge_model,
        )
    raise ValueError(f"Unsupported AI provider: {settings.provider}")
