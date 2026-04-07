"""Comprehensive test suite for LLMConfig.

Tests verify:
1. Configuration loading from environment variables
2. Tier-based model restrictions (DEVELOPMENT tier enforcement)
3. Cost estimates and monthly projections
4. Parameter validation (temperature, top_p ranges)
5. API key validation
6. Enum coercion and string handling
7. Default values
8. Error handling for invalid configurations
"""

import os
import pytest
from unittest.mock import patch
from decimal import Decimal

from modelado.config.llm_config import LLMConfig, LLMModel, LLMCostTier


class TestLLMModelEnum:
    """Test LLMModel enum values and properties."""
    
    def test_enum_values_defined(self):
        """Verify all required models are defined."""
        assert LLMModel.GPT_4O_MINI.value == "gpt-4o-mini"
        assert LLMModel.CLAUDE_35_HAIKU.value == "claude-3-5-haiku-20241022"
        assert LLMModel.GPT_4.value == "gpt-4"
        assert LLMModel.CLAUDE_3_OPUS.value == "claude-3-opus-20240229"
    
    def test_enum_count(self):
        """Verify all models are present."""
        assert len(LLMModel) == 4


class TestLLMCostTierEnum:
    """Test LLMCostTier enum values."""
    
    def test_enum_values_defined(self):
        """Verify all tiers are defined."""
        assert LLMCostTier.DEVELOPMENT.value == "development"
        assert LLMCostTier.STAGING.value == "staging"
        assert LLMCostTier.PRODUCTION.value == "production"
    
    def test_enum_count(self):
        """Verify all tiers are present."""
        assert len(LLMCostTier) == 3


class TestLLMConfigDefaults:
    """Test default configuration values."""
    
    def test_default_model_is_gpt4o_mini(self):
        """Verify default model is GPT-4o-mini (cheapest)."""
        config = LLMConfig()
        assert config.model == LLMModel.GPT_4O_MINI
    
    def test_default_tier_is_development(self):
        """Verify default tier is DEVELOPMENT (safest)."""
        config = LLMConfig()
        assert config.tier == LLMCostTier.DEVELOPMENT
    
    def test_default_rate_limiting(self):
        """Verify default rate limits."""
        config = LLMConfig()
        assert config.max_calls_per_minute == 30
        assert config.max_tokens_per_day == 100_000
    
    def test_default_generation_parameters(self):
        """Verify default generation parameters."""
        config = LLMConfig()
        assert config.temperature == 0.3  # Low for consistency
        assert config.max_tokens == 2000
        assert config.top_p == 0.95
    
    def test_default_safety_flags(self):
        """Verify default safety flags."""
        config = LLMConfig()
        assert config.enable_llm_extraction is True
        assert config.fallback_to_heuristics is True
        assert config.log_tokens is True
    
    def test_default_alert_threshold(self):
        """Verify default cost alert threshold."""
        config = LLMConfig()
        assert config.alert_cost_threshold == 50.0


class TestTierConstraintEnforcement:
    """Test that tier constraints are enforced at initialization."""
    
    def test_development_tier_allows_gpt4o_mini(self):
        """Verify GPT-4o-mini is allowed in development tier."""
        config = LLMConfig(
            model=LLMModel.GPT_4O_MINI,
            tier=LLMCostTier.DEVELOPMENT
        )
        assert config.model == LLMModel.GPT_4O_MINI
        assert config.tier == LLMCostTier.DEVELOPMENT
    
    def test_development_tier_allows_claude_haiku(self):
        """Verify Claude Haiku is allowed in development tier."""
        config = LLMConfig(
            model=LLMModel.CLAUDE_35_HAIKU,
            tier=LLMCostTier.DEVELOPMENT
        )
        assert config.model == LLMModel.CLAUDE_35_HAIKU
        assert config.tier == LLMCostTier.DEVELOPMENT
    
    def test_development_tier_rejects_gpt4(self):
        """Verify GPT-4 is rejected in development tier."""
        with pytest.raises(ValueError) as exc_info:
            LLMConfig(
                model=LLMModel.GPT_4,
                tier=LLMCostTier.DEVELOPMENT
            )
        assert "Development tier only supports" in str(exc_info.value)
        assert "gpt-4" in str(exc_info.value)
    
    def test_development_tier_rejects_claude_opus(self):
        """Verify Claude Opus is rejected in development tier."""
        with pytest.raises(ValueError) as exc_info:
            LLMConfig(
                model=LLMModel.CLAUDE_3_OPUS,
                tier=LLMCostTier.DEVELOPMENT
            )
        assert "Development tier only supports" in str(exc_info.value)
        assert "claude-3-opus" in str(exc_info.value)
    
    def test_staging_tier_allows_any_model(self):
        """Verify staging tier allows all models."""
        for model in LLMModel:
            config = LLMConfig(model=model, tier=LLMCostTier.STAGING)
            assert config.model == model
    
    def test_production_tier_allows_any_model(self):
        """Verify production tier allows all models."""
        for model in LLMModel:
            config = LLMConfig(model=model, tier=LLMCostTier.PRODUCTION)
            assert config.model == model


class TestEnvironmentVariableLoading:
    """Test loading configuration from environment variables."""
    
    def test_from_env_default_values(self):
        """Verify from_env() uses correct defaults when env vars not set."""
        with patch.dict(os.environ, {}, clear=True):
            config = LLMConfig.from_env()
            assert config.model == LLMModel.GPT_4O_MINI
            assert config.tier == LLMCostTier.DEVELOPMENT
            assert config.max_calls_per_minute == 30
            assert config.temperature == 0.3
    
    def test_from_env_loads_model(self):
        """Verify from_env() loads LLM_MODEL."""
        with patch.dict(os.environ, {"LLM_MODEL": "claude-3-5-haiku-20241022"}):
            config = LLMConfig.from_env()
            assert config.model == LLMModel.CLAUDE_35_HAIKU
    
    def test_from_env_loads_tier(self):
        """Verify from_env() loads LLM_TIER."""
        with patch.dict(os.environ, {"LLM_TIER": "staging"}):
            config = LLMConfig.from_env()
            assert config.tier == LLMCostTier.STAGING
    
    def test_from_env_loads_rate_limits(self):
        """Verify from_env() loads rate limit environment variables."""
        with patch.dict(os.environ, {
            "LLM_MAX_CALLS_PER_MIN": "60",
            "LLM_MAX_TOKENS_PER_DAY": "200000"
        }):
            config = LLMConfig.from_env()
            assert config.max_calls_per_minute == 60
            assert config.max_tokens_per_day == 200_000
    
    def test_from_env_loads_generation_parameters(self):
        """Verify from_env() loads generation parameters."""
        with patch.dict(os.environ, {
            "LLM_TEMPERATURE": "0.7",
            "LLM_MAX_TOKENS": "4000",
            "LLM_TOP_P": "0.8"
        }):
            config = LLMConfig.from_env()
            assert config.temperature == 0.7
            assert config.max_tokens == 4000
            assert config.top_p == 0.8
    
    def test_from_env_loads_safety_flags(self):
        """Verify from_env() loads safety flags."""
        with patch.dict(os.environ, {
            "LLM_EXTRACTION_ENABLED": "false",
            "LLM_FALLBACK_HEURISTICS": "false",
            "LLM_LOG_TOKENS": "false"
        }):
            config = LLMConfig.from_env()
            assert config.enable_llm_extraction is False
            assert config.fallback_to_heuristics is False
            assert config.log_tokens is False
    
    def test_from_env_loads_alert_threshold(self):
        """Verify from_env() loads alert cost threshold."""
        with patch.dict(os.environ, {"LLM_ALERT_COST": "100.0"}):
            config = LLMConfig.from_env()
            assert config.alert_cost_threshold == 100.0
    
    def test_from_env_case_insensitive_model(self):
        """Verify from_env() handles model names case-insensitively."""
        with patch.dict(os.environ, {"LLM_MODEL": "GPT-4O-MINI"}):
            config = LLMConfig.from_env()
            assert config.model == LLMModel.GPT_4O_MINI
    
    def test_from_env_case_insensitive_tier(self):
        """Verify from_env() handles tier names case-insensitively."""
        with patch.dict(os.environ, {"LLM_TIER": "PRODUCTION"}):
            config = LLMConfig.from_env()
            assert config.tier == LLMCostTier.PRODUCTION
    
    def test_from_env_invalid_model_raises(self):
        """Verify from_env() raises ValueError for invalid model."""
        with patch.dict(os.environ, {"LLM_MODEL": "invalid-model"}):
            with pytest.raises(ValueError) as exc_info:
                LLMConfig.from_env()
            assert "Invalid LLM_MODEL" in str(exc_info.value)
    
    def test_from_env_invalid_tier_raises(self):
        """Verify from_env() raises ValueError for invalid tier."""
        with patch.dict(os.environ, {"LLM_TIER": "invalid-tier"}):
            with pytest.raises(ValueError) as exc_info:
                LLMConfig.from_env()
            assert "Invalid LLM_TIER" in str(exc_info.value)
    
    def test_from_env_respects_tier_constraints(self):
        """Verify from_env() enforces tier constraints."""
        with patch.dict(os.environ, {
            "LLM_MODEL": "gpt-4",
            "LLM_TIER": "development"
        }):
            with pytest.raises(ValueError) as exc_info:
                LLMConfig.from_env()
            assert "Development tier only supports" in str(exc_info.value)


class TestParameterValidation:
    """Test parameter validation."""
    
    def test_temperature_zero_allowed(self):
        """Verify temperature=0 is allowed."""
        config = LLMConfig(temperature=0.0)
        assert config.temperature == 0.0
    
    def test_temperature_two_allowed(self):
        """Verify temperature=2.0 is allowed."""
        config = LLMConfig(temperature=2.0)
        assert config.temperature == 2.0
    
    def test_temperature_negative_rejected(self):
        """Verify negative temperature is rejected."""
        with pytest.raises(ValueError):
            LLMConfig(temperature=-0.1)
    
    def test_temperature_above_two_rejected(self):
        """Verify temperature > 2.0 is rejected."""
        with pytest.raises(ValueError):
            LLMConfig(temperature=2.1)
    
    def test_top_p_minimum_boundary(self):
        """Verify top_p > 0 is required."""
        with pytest.raises(ValueError):
            LLMConfig(top_p=0.0)
    
    def test_top_p_maximum_boundary(self):
        """Verify top_p <= 1.0 is required."""
        config = LLMConfig(top_p=1.0)
        assert config.top_p == 1.0
    
    def test_top_p_above_one_rejected(self):
        """Verify top_p > 1.0 is rejected."""
        with pytest.raises(ValueError):
            LLMConfig(top_p=1.1)
    
    def test_max_tokens_positive(self):
        """Verify max_tokens can be set to reasonable values."""
        config = LLMConfig(max_tokens=8000)
        assert config.max_tokens == 8000


class TestCostEstimates:
    """Test cost estimation calculations."""
    
    def test_gpt4o_mini_cost_estimate(self):
        """Verify GPT-4o-mini cost estimate."""
        config = LLMConfig(model=LLMModel.GPT_4O_MINI)
        cost = config.estimated_cost_per_1k_calls
        # ~$0.60 per 1000 calls (500 input @ $0.15/MTok, 800 output @ $0.60/MTok)
        assert 0.50 < cost < 0.75, f"Expected ~$0.60, got ${cost:.2f}"
    
    def test_claude_haiku_cost_estimate(self):
        """Verify Claude 3.5 Haiku cost estimate."""
        config = LLMConfig(model=LLMModel.CLAUDE_35_HAIKU)
        cost = config.estimated_cost_per_1k_calls
        # ~$0.0042 per 1000 calls (500 input @ $0.80/MTok, 800 output @ $4.00/MTok)
        # = (500*0.80 + 800*4.00)/1e6 = (400 + 3200)/1e6 = 3600/1e6 = $0.0036 per call
        # * 1000 calls = ~$3.60 per 1000 calls
        assert 3.0 < cost < 4.0, f"Expected ~$3.60, got ${cost:.2f}"
    
    def test_gpt4_cost_estimate(self):
        """Verify GPT-4 cost estimate (production tier)."""
        config = LLMConfig(model=LLMModel.GPT_4, tier=LLMCostTier.PRODUCTION)
        cost = config.estimated_cost_per_1k_calls
        # ~$63 per 1000 calls (500 @ $30/MTok, 800 @ $60/MTok)
        assert 60 < cost < 70, f"Expected ~$63, got ${cost:.2f}"
    
    def test_claude_opus_cost_estimate(self):
        """Verify Claude 3 Opus cost estimate (production tier)."""
        config = LLMConfig(model=LLMModel.CLAUDE_3_OPUS, tier=LLMCostTier.PRODUCTION)
        cost = config.estimated_cost_per_1k_calls
        # (500*15 + 800*75)/1e6 = (7500 + 60000)/1e6 = 67500/1e6 = $0.0675 per call
        # * 1000 calls = ~$67.50 per 1000 calls
        assert 65 < cost < 70, f"Expected ~$67.50, got ${cost:.2f}"
    
    def test_monthly_cost_projection(self):
        """Verify monthly cost projection calculation."""
        config = LLMConfig(model=LLMModel.GPT_4O_MINI)
        # 100 calls per day, 30 days per month
        monthly = config.estimated_monthly_cost
        assert 0 < monthly < 20, f"Monthly cost for mini model should be < $20"


class TestTierMethods:
    """Test tier checking methods."""
    
    def test_is_development_tier_true(self):
        """Verify is_development_tier() returns True for development tier."""
        config = LLMConfig(tier=LLMCostTier.DEVELOPMENT)
        assert config.is_development_tier() is True
    
    def test_is_development_tier_false(self):
        """Verify is_development_tier() returns False for non-development."""
        config = LLMConfig(tier=LLMCostTier.PRODUCTION, model=LLMModel.GPT_4)
        assert config.is_development_tier() is False
    
    def test_is_production_tier_true(self):
        """Verify is_production_tier() returns True for production tier."""
        config = LLMConfig(tier=LLMCostTier.PRODUCTION, model=LLMModel.GPT_4)
        assert config.is_production_tier() is True
    
    def test_is_production_tier_false(self):
        """Verify is_production_tier() returns False for non-production."""
        config = LLMConfig(tier=LLMCostTier.DEVELOPMENT)
        assert config.is_production_tier() is False


class TestValidationMethod:
    """Test the check_config() method."""
    
    def test_check_config_with_extraction_disabled(self):
        """Verify check_config() passes when extraction disabled."""
        config = LLMConfig(enable_llm_extraction=False)
        assert config.check_config() is True
    
    def test_check_config_with_openai_key_set(self):
        """Verify check_config() passes with OPENAI_API_KEY set."""
        with patch.dict(os.environ, {"OPENAI_API_KEY": "sk-test-123"}):
            config = LLMConfig(model=LLMModel.GPT_4O_MINI, enable_llm_extraction=True)
            assert config.check_config() is True
    
    def test_check_config_missing_openai_key_raises(self):
        """Verify check_config() raises if OpenAI key missing but needed."""
        with patch.dict(os.environ, {}, clear=True):
            config = LLMConfig(model=LLMModel.GPT_4O_MINI, enable_llm_extraction=True)
            with pytest.raises(ValueError) as exc_info:
                config.check_config()
            assert "OPENAI_API_KEY" in str(exc_info.value)
    
    def test_check_config_with_anthropic_key_set(self):
        """Verify check_config() passes with ANTHROPIC_API_KEY set."""
        with patch.dict(os.environ, {"ANTHROPIC_API_KEY": "sk-ant-test-123"}):
            config = LLMConfig(model=LLMModel.CLAUDE_35_HAIKU, enable_llm_extraction=True)
            assert config.check_config() is True
    
    def test_check_config_missing_anthropic_key_raises(self):
        """Verify check_config() raises if Anthropic key missing but needed."""
        with patch.dict(os.environ, {}, clear=True):
            config = LLMConfig(model=LLMModel.CLAUDE_35_HAIKU, enable_llm_extraction=True)
            with pytest.raises(ValueError) as exc_info:
                config.check_config()
            assert "ANTHROPIC_API_KEY" in str(exc_info.value)


class TestTaskModelSelector:
    """Test the TaskModelSelector class."""
    
    def test_development_tier_always_mini(self):
        """Verify dev tier always gets mini model regardless of task."""
        from modelado.config.llm_config import TaskModelSelector, LLMTask
        
        for task in LLMTask:
            config = TaskModelSelector.get_config(task, LLMCostTier.DEVELOPMENT)
            assert config.model == LLMModel.GPT_4O_MINI
            assert config.tier == LLMCostTier.DEVELOPMENT
            
    def test_production_directive_gets_gpt4(self):
        """Verify production directive gets GPT-4."""
        from modelado.config.llm_config import TaskModelSelector, LLMTask
        
        config = TaskModelSelector.get_config(LLMTask.DIRECTIVE, LLMCostTier.PRODUCTION)
        assert config.model == LLMModel.GPT_4
        assert config.tier == LLMCostTier.PRODUCTION
        
    def test_production_evaluation_gets_gpt4(self):
        """Verify production evaluation gets GPT-4."""
        from modelado.config.llm_config import TaskModelSelector, LLMTask
        
        config = TaskModelSelector.get_config(LLMTask.EVALUATION, LLMCostTier.PRODUCTION)
        assert config.model == LLMModel.GPT_4
        assert config.tier == LLMCostTier.PRODUCTION
        
    def test_production_lifting_gets_mini(self):
        """Verify production lifting still gets mini for efficiency."""
        from modelado.config.llm_config import TaskModelSelector, LLMTask
        
        config = TaskModelSelector.get_config(LLMTask.LIFTING, LLMCostTier.PRODUCTION)
        assert config.model == LLMModel.GPT_4O_MINI
        assert config.tier == LLMCostTier.PRODUCTION

    def test_production_task_matrix_strict_routing(self):
        """Verify all production tasks route deterministically by policy."""
        from modelado.config.llm_config import TaskModelSelector, LLMTask

        expected_gpt4 = {LLMTask.DIRECTIVE, LLMTask.EVALUATION}
        for task in LLMTask:
            config = TaskModelSelector.get_config(task, LLMCostTier.PRODUCTION)
            if task in expected_gpt4:
                assert config.model == LLMModel.GPT_4
            else:
                assert config.model == LLMModel.GPT_4O_MINI
            assert config.tier == LLMCostTier.PRODUCTION

    def test_staging_task_matrix_matches_production_policy(self):
        """Verify staging uses the same task routing policy as production."""
        from modelado.config.llm_config import TaskModelSelector, LLMTask

        expected_gpt4 = {LLMTask.DIRECTIVE, LLMTask.EVALUATION}
        for task in LLMTask:
            config = TaskModelSelector.get_config(task, LLMCostTier.STAGING)
            if task in expected_gpt4:
                assert config.model == LLMModel.GPT_4
            else:
                assert config.model == LLMModel.GPT_4O_MINI
            assert config.tier == LLMCostTier.STAGING


class TestRealWorldScenarios:
    """Test realistic usage scenarios."""
    
    def test_rapid_prototyping_scenario(self):
        """Simulate rapid prototyping setup."""
        with patch.dict(os.environ, {
            "LLM_MODEL": "gpt-4o-mini",
            "LLM_TIER": "development",
            "LLM_TEMPERATURE": "0.3",
            "OPENAI_API_KEY": "sk-test-123"
        }):
            config = LLMConfig.from_env()
            
            # Verify it's safe for development
            assert config.is_development_tier()
            assert config.model == LLMModel.GPT_4O_MINI
            assert config.temperature == 0.3
            
            # Validate API key exists
            assert config.check_config() is True
            
            # Check cost is reasonable for daily development
            monthly_cost = config.estimated_monthly_cost
            assert monthly_cost < 50, "Development should cost < $50/month"
    
    def test_production_scenario(self):
        """Simulate production setup with higher-quality model."""
        with patch.dict(os.environ, {
            "LLM_MODEL": "gpt-4",
            "LLM_TIER": "production",
            "LLM_TEMPERATURE": "0.2",
            "LLM_MAX_CALLS_PER_MIN": "100",
            "OPENAI_API_KEY": "sk-prod-123"
        }):
            config = LLMConfig.from_env()
            
            # Verify it's production-grade
            assert config.is_production_tier()
            assert config.model == LLMModel.GPT_4
            assert config.max_calls_per_minute == 100
            
            # Higher quality (lower temperature)
            assert config.temperature == 0.2
            
            # Cost estimate shows higher spend
            cost_per_1k = config.estimated_cost_per_1k_calls
            assert cost_per_1k > 50, "GPT-4 should cost much more than mini"


class TestConfigImmutability:
    """Test that config behaves correctly (quasi-immutable from Pydantic)."""
    
    def test_config_fields_settable_at_init(self):
        """Verify all fields can be set at initialization."""
        config = LLMConfig(
            model=LLMModel.CLAUDE_35_HAIKU,
            tier=LLMCostTier.DEVELOPMENT,
            temperature=0.5,
            max_tokens=4000
        )
        
        assert config.model == LLMModel.CLAUDE_35_HAIKU
        assert config.temperature == 0.5
        assert config.max_tokens == 4000


class TestEdgeCases:
    """Test edge cases and boundary conditions."""
    
    def test_empty_environment_uses_defaults(self):
        """Verify empty environment uses safe defaults."""
        with patch.dict(os.environ, {}, clear=True):
            config = LLMConfig.from_env()
            
            # Should use cheapest, safest model
            assert config.model == LLMModel.GPT_4O_MINI
            assert config.tier == LLMCostTier.DEVELOPMENT
    
    def test_partial_environment_uses_defaults(self):
        """Verify partial environment fills in defaults."""
        with patch.dict(os.environ, {"LLM_MODEL": "claude-3-5-haiku-20241022"}, clear=True):
            config = LLMConfig.from_env()
            
            # Model from env, tier from default
            assert config.model == LLMModel.CLAUDE_35_HAIKU
            assert config.tier == LLMCostTier.DEVELOPMENT  # Safe default
    
    def test_whitespace_in_environment_values(self):
        """Verify whitespace handling in environment values."""
        with patch.dict(os.environ, {"LLM_MODEL": "  gpt-4o-mini  ", "LLM_TIER": "  development  "}):
            config = LLMConfig.from_env()
            assert config.model == LLMModel.GPT_4O_MINI
            assert config.tier == LLMCostTier.DEVELOPMENT
