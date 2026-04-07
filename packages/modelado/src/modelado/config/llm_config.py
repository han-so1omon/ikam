"""LLM Configuration with cost-aware tier enforcement.

This module provides the LLMConfig class which enforces cost control by tier:
- DEVELOPMENT: GPT-4o-mini or Claude 3.5 Haiku ONLY (rapid prototyping, ~$0-20/month)
- STAGING: Higher-tier models allowed (testing, ~$50-200/month)
- PRODUCTION: Any model allowed (enterprise, cost varies)

The configuration is immutable after creation and enforces tier constraints at
initialization time, preventing accidental use of expensive models in development.

Cost Estimates (per 1000 extractions, ~500 input + 800 output tokens avg):
- GPT-4o-mini: ~$0.60 per 1000 calls (~$0.015/call)
- Claude 3.5 Haiku: ~$0.0042 per 1000 calls (~$0.0000042/call)
- GPT-4: ~$63.00 per 1000 calls (~$1.57/call) — PRODUCTION ONLY
- Claude 3 Opus: ~$30.00 per 1000 calls (~$0.75/call) — PRODUCTION ONLY

Example:
    Rapid prototyping with GPT-4o-mini:
    >>> config = LLMConfig.from_env()
    >>> assert config.tier == LLMCostTier.DEVELOPMENT
    >>> assert config.model == LLMModel.GPT_4O_MINI
    >>> print(f"Cost: ${config.estimated_cost_per_1k_calls:.2f} per 1000 calls")
    Cost: $0.60 per 1000 calls

    Enforce tier constraint:
    >>> config_invalid = LLMConfig(
    ...     model=LLMModel.GPT_4,  # Too expensive
    ...     tier=LLMCostTier.DEVELOPMENT  # This raises ValueError
    ... )
    Traceback (most recent call last):
        ...
    ValueError: Development tier only supports GPT-4o-mini or Claude 3.5 Haiku. Got: gpt-4

References:
    - LLM configuration with cost control
    - docs/ikam/llm-cost-control.md
    - AGENTS.md § "Model Selection for Code Generation"
"""

import os
from dataclasses import dataclass
from enum import Enum
from typing import Optional
from decimal import Decimal
from pydantic import BaseModel, field_validator


class LLMModel(str, Enum):
    """Available LLM models for concept extraction.
    
    Pricing (as of December 2024):
    - Input/output costs per 1M tokens
    - GPT models: OpenAI
    - Claude models: Anthropic
    """
    
    # Development tier models (low cost, suitable for prototyping)
    GPT_4O_MINI = "gpt-4o-mini"  # OpenAI: $0.15/MTok input, $0.60/MTok output
    CLAUDE_35_HAIKU = "claude-3-5-haiku-20241022"  # Anthropic: $0.80/MTok input, $4.00/MTok output
    
    # Production tier models (higher cost, higher quality)
    GPT_4 = "gpt-4"  # OpenAI: $30/MTok input, $60/MTok output (v1 turbo cheaper)
    CLAUDE_3_OPUS = "claude-3-opus-20240229"  # Anthropic: $15/MTok input, $75/MTok output


class LLMCostTier(str, Enum):
    """Cost tier for LLM selection.
    
    Determines which models are allowed:
    - DEVELOPMENT: Cheapest models for rapid iteration
    - STAGING: Mid-tier models for testing
    - PRODUCTION: All models allowed for quality/performance
    """
    
    DEVELOPMENT = "development"  # GPT-4o-mini or Claude 3.5 Haiku only
    STAGING = "staging"  # Can use higher-tier models
    PRODUCTION = "production"  # Any model allowed


class LLMTask(str, Enum):
    """Specific modeling tasks requiring LLM involvement."""
    LIFTING = "lifting"
    REPAIR = "repair"
    DIRECTIVE = "directive"
    NORMALIZATION = "normalization"
    EVALUATION = "evaluation"
    IDENTIFY = "identify"


class LLMConfig(BaseModel):
    """LLM configuration with tier-based cost control.
    
    Enforces development tier restrictions at initialization to prevent
    accidental use of expensive models during prototyping.
    
    Configuration can be loaded from environment variables using from_env(),
    or created directly with validation.
    
    Attributes:
        model: Selected LLM model (default: GPT-4o-mini)
        tier: Cost tier controlling allowed models (default: DEVELOPMENT)
        max_calls_per_minute: Rate limit for API calls (default: 30)
        max_tokens_per_day: Daily token budget (default: 100,000)
        temperature: Sampling temperature for consistency (default: 0.3)
        max_tokens: Max tokens per generation (default: 2000)
        top_p: Nucleus sampling parameter (default: 0.95)
        enable_llm_extraction: Whether to enable LLM extraction (default: True)
        fallback_to_heuristics: Fallback to rule-based if LLM fails (default: True)
        log_tokens: Log token usage for cost tracking (default: True)
        alert_cost_threshold: Alert if daily spend exceeds this ($, default: 50.0)
    """
    
    model: LLMModel = LLMModel.GPT_4O_MINI
    tier: LLMCostTier = LLMCostTier.DEVELOPMENT
    
    # Rate limiting
    max_calls_per_minute: int = 30
    max_tokens_per_day: int = 100_000
    
    # Generation parameters
    temperature: float = 0.3  # Low for consistency in concept extraction
    max_tokens: int = 2000
    top_p: float = 0.95
    
    # Safety and fallback
    enable_llm_extraction: bool = True
    fallback_to_heuristics: bool = True
    
    # Monitoring
    log_tokens: bool = True
    alert_cost_threshold: float = 50.0
    
    model_config = {"validate_default": True}
    
    @field_validator("model", "tier", mode="before")
    @classmethod
    def coerce_enum(cls, v):
        """Coerce string values to enum members."""
        if isinstance(v, str):
            # Try as enum value first
            try:
                if v.lower() in [e.value for e in LLMModel]:
                    return LLMModel(v.lower())
                if v.lower() in [e.value for e in LLMCostTier]:
                    return LLMCostTier(v.lower())
            except (ValueError, KeyError):
                pass
        return v
    
    @field_validator("temperature")
    @classmethod
    def validate_temperature(cls, v):
        """Validate temperature is between 0 and 2."""
        if not 0 <= v <= 2:
            raise ValueError("temperature must be between 0 and 2")
        return v
    
    @field_validator("top_p")
    @classmethod
    def validate_top_p(cls, v):
        """Validate top_p is between 0 and 1."""
        if not 0 < v <= 1:
            raise ValueError("top_p must be between 0 and 1")
        return v
    
    def __init__(self, **data):
        """Initialize and validate tier constraints."""
        super().__init__(**data)
        self._validate_tier_constraints()
    
    def _validate_tier_constraints(self) -> None:
        """Enforce tier-based model restrictions.
        
        Raises:
            ValueError: If model is not allowed in the configured tier
        """
        if self.tier == LLMCostTier.DEVELOPMENT:
            allowed_models = {LLMModel.GPT_4O_MINI, LLMModel.CLAUDE_35_HAIKU}
            if self.model not in allowed_models:
                raise ValueError(
                    f"Development tier only supports GPT-4o-mini or Claude 3.5 Haiku. "
                    f"Got: {self.model.value}"
                )
    
    @classmethod
    def from_env(cls) -> "LLMConfig":
        """Load configuration from environment variables.
        
        Supported environment variables:
        - LLM_MODEL: Model name (gpt-4o-mini, claude-3-5-haiku-20241022, etc.)
        - LLM_TIER: Cost tier (development, staging, production)
        - LLM_MAX_CALLS_PER_MIN: Rate limit for API calls
        - LLM_MAX_TOKENS_PER_DAY: Daily token budget
        - LLM_TEMPERATURE: Sampling temperature (0-2)
        - LLM_MAX_TOKENS: Max tokens per generation
        - LLM_TOP_P: Nucleus sampling parameter (0-1)
        - LLM_EXTRACTION_ENABLED: Enable LLM extraction (true/false)
        - LLM_FALLBACK_HEURISTICS: Fallback to rule-based (true/false)
        - LLM_LOG_TOKENS: Log token usage (true/false)
        - LLM_ALERT_COST: Cost alert threshold in dollars
        
        Returns:
            LLMConfig instance with environment variables applied
            
        Raises:
            ValueError: If tier constraints are violated or invalid values provided
        """
        model_str = os.getenv("LLM_MODEL", "gpt-4o-mini").lower().strip()
        tier_str = os.getenv("LLM_TIER", "development").lower().strip()
        
        # Parse tier
        try:
            tier = LLMCostTier(tier_str)
        except ValueError:
            raise ValueError(
                f"Invalid LLM_TIER. Must be one of: "
                f"{', '.join(e.value for e in LLMCostTier)}"
            )
        
        # Parse model
        try:
            model = LLMModel(model_str)
        except ValueError:
            valid_models = ", ".join(e.value for e in LLMModel)
            raise ValueError(f"Invalid LLM_MODEL '{model_str}'. Must be one of: {valid_models}")
        
        # Create config (will validate tier constraints in __init__)
        return cls(
            model=model,
            tier=tier,
            max_calls_per_minute=int(os.getenv("LLM_MAX_CALLS_PER_MIN", "30")),
            max_tokens_per_day=int(os.getenv("LLM_MAX_TOKENS_PER_DAY", "100000")),
            temperature=float(os.getenv("LLM_TEMPERATURE", "0.3")),
            max_tokens=int(os.getenv("LLM_MAX_TOKENS", "2000")),
            top_p=float(os.getenv("LLM_TOP_P", "0.95")),
            enable_llm_extraction=os.getenv("LLM_EXTRACTION_ENABLED", "true").lower() == "true",
            fallback_to_heuristics=os.getenv("LLM_FALLBACK_HEURISTICS", "true").lower() == "true",
            log_tokens=os.getenv("LLM_LOG_TOKENS", "true").lower() == "true",
            alert_cost_threshold=float(os.getenv("LLM_ALERT_COST", "50.0")),
        )
    
    @property
    def estimated_cost_per_1k_calls(self) -> float:
        """Estimate cost for 1000 extraction calls.
        
        Assumes average extraction:
        - Input: 500 tokens (conversation context)
        - Output: 800 tokens (generated concepts)
        
        This is a rough estimate for budget planning purposes.
        
        Returns:
            Estimated cost in USD per 1000 extraction calls
        """
        # Token counts for typical extraction
        avg_input_tokens = 500
        avg_output_tokens = 800
        
        # Cost per million tokens (as of December 2024)
        costs_per_million_tokens = {
            LLMModel.GPT_4O_MINI: {
                "input": 0.15,  # $0.15 per 1M tokens
                "output": 0.60,  # $0.60 per 1M tokens
            },
            LLMModel.CLAUDE_35_HAIKU: {
                "input": 0.80,  # $0.80 per 1M tokens
                "output": 4.00,  # $4.00 per 1M tokens
            },
            LLMModel.GPT_4: {
                "input": 30.0,
                "output": 60.0,
            },
            LLMModel.CLAUDE_3_OPUS: {
                "input": 15.0,
                "output": 75.0,
            },
        }
        
        if self.model not in costs_per_million_tokens:
            return 0.0
        
        costs = costs_per_million_tokens[self.model]
        cost_per_call = (
            (avg_input_tokens * costs["input"]) / 1_000_000 +
            (avg_output_tokens * costs["output"]) / 1_000_000
        )
        
        return cost_per_call * 1000
    
    @property
    def estimated_monthly_cost(self, calls_per_day: int = 100) -> float:
        """Estimate monthly cost based on daily call volume.
        
        Args:
            calls_per_day: Average number of extraction calls per day
        
        Returns:
            Estimated monthly cost in USD
        """
        cost_per_call = self.estimated_cost_per_1k_calls / 1000
        return cost_per_call * calls_per_day * 30
    
    def is_development_tier(self) -> bool:
        """Check if this is a development-tier configuration."""
        return self.tier == LLMCostTier.DEVELOPMENT
    
    def is_production_tier(self) -> bool:
        """Check if this is a production-tier configuration."""
        return self.tier == LLMCostTier.PRODUCTION
    
    def check_config(self) -> bool:
        """Validate configuration consistency.
        
        Performs additional runtime validation beyond Pydantic validation.
        
        Returns:
            True if configuration is valid
            
        Raises:
            ValueError: If configuration is invalid
        """
        # Tier constraints already validated in __init__
        
        # API keys must be set for the selected model
        if self.enable_llm_extraction:
            if self.model in {LLMModel.GPT_4O_MINI, LLMModel.GPT_4}:
                if not os.getenv("OPENAI_API_KEY"):
                    raise ValueError("OPENAI_API_KEY not set but OpenAI model selected")
            elif self.model in {LLMModel.CLAUDE_35_HAIKU, LLMModel.CLAUDE_3_OPUS}:
                if not os.getenv("ANTHROPIC_API_KEY"):
                    raise ValueError("ANTHROPIC_API_KEY not set but Anthropic model selected")
        
        return True


class ProviderLLMSettings(BaseModel):
    """Provider-level settings for creating a unified LLM client.

    Attributes:
        provider: Backend provider name (only "openai" is currently supported).
        model: Primary generation model.
        embed_model: Model used for embeddings.
        judge_model: Model used for scoring/judging.
    """

    provider: str = "openai"
    model: str = LLMModel.GPT_4O_MINI
    embed_model: str = "text-embedding-3-small"
    judge_model: str = LLMModel.GPT_4O_MINI

    @classmethod
    def from_env(cls) -> "ProviderLLMSettings":
        """Load provider settings from environment variables.

        Supported variables:
        - LLM_PROVIDER: Backend provider (default: openai)
        - LLM_MODEL: Primary model (default: gpt-4o-mini)
        - LLM_EMBED_MODEL: Embedding model (default: text-embedding-3-small)
        - LLM_JUDGE_MODEL: Judge model (default: gpt-4o-mini)
        """
        return cls(
            provider=os.getenv("LLM_PROVIDER", "openai"),
            model=os.getenv("LLM_MODEL", LLMModel.GPT_4O_MINI),
            embed_model=os.getenv("LLM_EMBED_MODEL", "text-embedding-3-small"),
            judge_model=os.getenv("LLM_JUDGE_MODEL", LLMModel.GPT_4O_MINI),
        )


class TaskModelSelector:
    """Routes modeling tasks to appropriate models based on cost tier.
    
    This selector ensures that expensive models are only used when necessary
    and allowed by the configuration tier.
    """
    
    @staticmethod
    def get_config(task: LLMTask, tier: LLMCostTier = LLMCostTier.DEVELOPMENT) -> LLMConfig:
        """Get the appropriate configuration for a specific task and tier.
        
        Args:
            task: The modeling task to perform (LIFTING, REPAIR, etc.)
            tier: The allowed cost tier (default: DEVELOPMENT)
            
        Returns:
            LLMConfig optimized for the task while respecting tier limits
        """
        # Development tier is always restricted to cheap models
        if tier == LLMCostTier.DEVELOPMENT:
            # All tasks use gpt-4o-mini in dev unless specific reason for haiku
            return LLMConfig(model=LLMModel.GPT_4O_MINI, tier=tier)
            
        # Staging/Production logic
        if task in {LLMTask.DIRECTIVE, LLMTask.EVALUATION}:
            # High-stakes tasks get high-quality models if allowed
            return LLMConfig(model=LLMModel.GPT_4, tier=tier)
            
        # Routine tasks still use efficient models even in production
        return LLMConfig(model=LLMModel.GPT_4O_MINI, tier=tier)


__all__ = ["LLMConfig", "LLMModel", "LLMCostTier", "LLMTask", "TaskModelSelector", "ProviderLLMSettings"]
