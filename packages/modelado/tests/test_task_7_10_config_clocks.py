"""Tests for configuration management and pluggable clocks (Task 9.7.10).

Tests cover:
- TraversalConfig validation and environment loading
- TimingConfig validation and environment loading
- StepClock deterministic control
- WallClock production timing
- ClockFactory test/production mode switching
- TraversalEngine clock injection
"""

import os
import pytest
import asyncio
from unittest.mock import patch, MagicMock

from modelado.core.config import (
    TraversalConfig, TimingConfig, StepClock, WallClock, ClockFactory,
    get_traversal_config, get_timing_config, reset_config
)


class TestTraversalConfig:
    """Test traversal configuration."""
    
    def test_default_values(self):
        """Test default configuration values."""
        config = TraversalConfig()
        assert config.enable_spike_decay is True
        assert config.base_step_duration_ms == 100
        assert config.spike_multiplier == 2.0
        assert config.decay_half_life_steps == 10
        assert config.min_step_duration_ms == 10
        assert config.max_step_duration_ms == 5000
    
    def test_load_from_env(self):
        """Test loading configuration from environment variables."""
        env_vars = {
            "TRAVERSAL_ENABLE_SPIKE_DECAY": "false",
            "TRAVERSAL_BASE_STEP_DURATION_MS": "200",
            "TRAVERSAL_SPIKE_MULTIPLIER": "3.0",
            "TRAVERSAL_DECAY_HALF_LIFE_STEPS": "5",
            "TRAVERSAL_MIN_STEP_DURATION_MS": "20",
            "TRAVERSAL_MAX_STEP_DURATION_MS": "10000",
        }
        
        with patch.dict(os.environ, env_vars):
            config = TraversalConfig.from_env()
            assert config.enable_spike_decay is False
            assert config.base_step_duration_ms == 200
            assert config.spike_multiplier == 3.0
            assert config.decay_half_life_steps == 5
            assert config.min_step_duration_ms == 20
            assert config.max_step_duration_ms == 10000
    
    def test_validate_valid_config(self):
        """Test validation of valid configuration."""
        config = TraversalConfig()
        config.validate()  # Should not raise
    
    def test_validate_invalid_base_step(self):
        """Test validation rejects invalid base step duration."""
        config = TraversalConfig(base_step_duration_ms=0)
        with pytest.raises(ValueError, match="base_step_duration_ms must be > 0"):
            config.validate()
    
    def test_validate_invalid_spike_multiplier(self):
        """Test validation rejects invalid spike multiplier."""
        config = TraversalConfig(spike_multiplier=0.5)
        with pytest.raises(ValueError, match="spike_multiplier must be >= 1.0"):
            config.validate()
    
    def test_validate_invalid_decay_half_life(self):
        """Test validation rejects invalid decay half-life."""
        config = TraversalConfig(decay_half_life_steps=0)
        with pytest.raises(ValueError, match="decay_half_life_steps must be > 0"):
            config.validate()
    
    def test_validate_invalid_min_max(self):
        """Test validation rejects invalid min/max range."""
        config = TraversalConfig(
            min_step_duration_ms=5000,
            max_step_duration_ms=100
        )
        with pytest.raises(ValueError, match="max_step_duration_ms must be >= min_step_duration_ms"):
            config.validate()


class TestTimingConfig:
    """Test timing configuration."""
    
    def test_default_values(self):
        """Test default timing configuration values."""
        config = TimingConfig()
        assert config.enable_metrics is True
        assert config.sample_rate == 1.0
        assert config.bucket_count == 10
    
    def test_load_from_env(self):
        """Test loading timing configuration from environment."""
        env_vars = {
            "TIMING_ENABLE_METRICS": "false",
            "TIMING_SAMPLE_RATE": "0.5",
            "TIMING_BUCKET_COUNT": "20",
        }
        
        with patch.dict(os.environ, env_vars):
            config = TimingConfig.from_env()
            assert config.enable_metrics is False
            assert config.sample_rate == 0.5
            assert config.bucket_count == 20
    
    def test_validate_valid_config(self):
        """Test validation of valid timing config."""
        config = TimingConfig()
        config.validate()  # Should not raise
    
    def test_validate_invalid_sample_rate_high(self):
        """Test validation rejects sample rate > 1.0."""
        config = TimingConfig(sample_rate=1.5)
        with pytest.raises(ValueError, match="sample_rate must be between 0.0 and 1.0"):
            config.validate()
    
    def test_validate_invalid_sample_rate_low(self):
        """Test validation rejects sample rate < 0.0."""
        config = TimingConfig(sample_rate=-0.5)
        with pytest.raises(ValueError, match="sample_rate must be between 0.0 and 1.0"):
            config.validate()
    
    def test_validate_invalid_bucket_count(self):
        """Test validation rejects invalid bucket count."""
        config = TimingConfig(bucket_count=0)
        with pytest.raises(ValueError, match="bucket_count must be > 0"):
            config.validate()


class TestStepClock:
    """Test step-based clock for deterministic testing."""
    
    def test_initialization(self):
        """Test step clock initialization."""
        clock = StepClock(initial_step=0)
        assert clock.current_step == 0
        assert clock.step_duration_ms == 100
    
    def test_initialization_with_offset(self):
        """Test step clock with non-zero initial step."""
        clock = StepClock(initial_step=5)
        assert clock.current_step == 5
    
    def test_tick(self):
        """Test step clock tick operation."""
        clock = StepClock()
        assert clock.tick() == 1
        assert clock.tick() == 2
        assert clock.tick() == 3
    
    def test_current_time_ms(self):
        """Test step clock millisecond conversion."""
        clock = StepClock()
        clock.step_duration_ms = 100
        
        assert clock.current_time_ms() == 0.0
        clock.tick()
        assert clock.current_time_ms() == 100.0
        clock.tick()
        assert clock.current_time_ms() == 200.0
    
    def test_advance_ms(self):
        """Test advancing clock by milliseconds."""
        clock = StepClock()
        clock.step_duration_ms = 100
        
        clock.advance_ms(250)  # 2.5 steps → rounded to 2 steps
        assert clock.current_step == 2
    
    def test_advance_ms_minimum(self):
        """Test advance_ms enforces minimum 1 step."""
        clock = StepClock()
        clock.step_duration_ms = 100
        
        clock.advance_ms(10)  # <1 step → rounds to 1
        assert clock.current_step == 1
    
    def test_reset(self):
        """Test step clock reset."""
        clock = StepClock()
        clock.tick()
        clock.tick()
        assert clock.current_step == 2
        
        clock.reset()
        assert clock.current_step == 0
    
    def test_deterministic_timing(self):
        """Test step clock provides deterministic timing."""
        clock1 = StepClock()
        clock2 = StepClock()
        
        for _ in range(5):
            clock1.tick()
            clock2.tick()
        
        assert clock1.current_time_ms() == clock2.current_time_ms()


class TestWallClock:
    """Test production wall-clock timing."""
    
    def test_initialization(self):
        """Test wall clock initializes without error."""
        clock = WallClock()
        assert hasattr(clock, 'start_time_ns')
    
    def test_current_time_increases(self):
        """Test wall clock time increases monotonically."""
        clock = WallClock()
        time1 = clock.current_time_ms()
        
        # Small sleep to ensure time advances
        import time
        time.sleep(0.01)  # 10ms
        
        time2 = clock.current_time_ms()
        assert time2 >= time1
    
    def test_tick_returns_elapsed_time(self):
        """Test tick returns elapsed milliseconds."""
        clock = WallClock()
        
        import time
        time.sleep(0.01)  # 10ms
        
        elapsed = clock.tick()
        assert elapsed >= 10  # At least 10ms
    
    def test_reset(self):
        """Test wall clock reset."""
        clock = WallClock()
        
        import time
        time.sleep(0.02)  # 20ms to ensure time difference
        
        time1 = clock.current_time_ms()
        clock.reset()
        time2 = clock.current_time_ms()
        
        assert time2 <= time1  # After reset, time is at least smaller or same


class TestClockFactory:
    """Test clock factory for test/production mode switching."""
    
    def teardown_method(self):
        """Reset factory to production mode after each test."""
        ClockFactory.set_test_mode(False)
    
    def test_production_mode_default(self):
        """Test factory creates WallClock by default."""
        clock = ClockFactory.create()
        assert isinstance(clock, WallClock)
    
    def test_test_mode_creation(self):
        """Test factory creates StepClock in test mode."""
        ClockFactory.set_test_mode(True)
        clock = ClockFactory.create()
        assert isinstance(clock, StepClock)
    
    def test_test_mode_toggle(self):
        """Test toggling between test and production modes."""
        ClockFactory.set_test_mode(True)
        assert isinstance(ClockFactory.create(), StepClock)
        
        ClockFactory.set_test_mode(False)
        assert isinstance(ClockFactory.create(), WallClock)
    
    def test_multiple_instances_independent(self):
        """Test multiple clock instances are independent."""
        ClockFactory.set_test_mode(True)
        clock1 = ClockFactory.create()
        clock2 = ClockFactory.create()
        
        clock1.tick()
        clock1.tick()
        
        assert clock1.current_step == 2
        assert clock2.current_step == 0


class TestGlobalConfig:
    """Test global configuration caching."""
    
    def teardown_method(self):
        """Reset global config after each test."""
        reset_config()
    
    def test_traversal_config_caching(self):
        """Test traversal config is cached."""
        config1 = get_traversal_config()
        config2 = get_traversal_config()
        assert config1 is config2
    
    def test_timing_config_caching(self):
        """Test timing config is cached."""
        config1 = get_timing_config()
        config2 = get_timing_config()
        assert config1 is config2
    
    def test_reset_config_clears_cache(self):
        """Test reset_config clears cached configs."""
        config1 = get_traversal_config()
        reset_config()
        config2 = get_traversal_config()
        assert config1 is not config2


class TestTraversalEngineClockInjection:
    """Test clock injection into TraversalEngine."""
    
    @pytest.mark.asyncio
    async def test_traversal_engine_accepts_clock(self):
        """Test TraversalEngine accepts injected clock."""
        from modelado.core.traversal_engine import TraversalEngine
        
        mock_graph = MagicMock()
        clock = StepClock()
        
        engine = TraversalEngine(mock_graph, clock=clock)
        assert engine.clock is clock
    
    @pytest.mark.asyncio
    async def test_traversal_engine_default_clock(self):
        """Test TraversalEngine uses factory default when no clock provided."""
        from modelado.core.traversal_engine import TraversalEngine
        
        mock_graph = MagicMock()
        
        ClockFactory.set_test_mode(False)
        engine = TraversalEngine(mock_graph)
        assert isinstance(engine.clock, WallClock)
        
        ClockFactory.set_test_mode(False)  # Reset
    
    def test_clock_injection_workflow(self):
        """Test complete clock injection workflow for testing."""
        from modelado.core.traversal_engine import TraversalEngine
        
        # Setup
        ClockFactory.set_test_mode(True)
        mock_graph = MagicMock()
        test_clock = ClockFactory.create()
        
        # Create engine with test clock
        engine = TraversalEngine(mock_graph, clock=test_clock)
        
        # Verify clock can be controlled for deterministic testing
        assert test_clock.current_step == 0
        test_clock.tick()
        assert test_clock.current_step == 1
        assert engine.clock.current_step == 1
        
        # Cleanup
        ClockFactory.set_test_mode(False)


class TestConfigIntegration:
    """Integration tests for configuration system."""
    
    def teardown_method(self):
        """Cleanup after each test."""
        reset_config()
        ClockFactory.set_test_mode(False)
    
    def test_full_config_workflow(self):
        """Test complete configuration and clock workflow."""
        # Setup environment
        env_vars = {
            "TRAVERSAL_ENABLE_SPIKE_DECAY": "true",
            "TRAVERSAL_BASE_STEP_DURATION_MS": "150",
            "TIMING_ENABLE_METRICS": "true",
            "TIMING_SAMPLE_RATE": "0.8",
        }
        
        with patch.dict(os.environ, env_vars):
            # Reset to force reload from env
            reset_config()
            
            # Load configs
            traversal_config = get_traversal_config()
            timing_config = get_timing_config()
            
            # Verify
            assert traversal_config.base_step_duration_ms == 150
            assert timing_config.sample_rate == 0.8
            
            # Create clock
            ClockFactory.set_test_mode(True)
            clock = ClockFactory.create()
            assert isinstance(clock, StepClock)
    
    def test_mixed_prod_test_environment(self):
        """Test mixing production and test configurations."""
        # Start in production
        assert isinstance(ClockFactory.create(), WallClock)
        
        # Switch to test
        ClockFactory.set_test_mode(True)
        test_clock = ClockFactory.create()
        assert isinstance(test_clock, StepClock)
        
        # Control test clock deterministically
        test_clock.tick()
        test_clock.tick()
        assert test_clock.current_step == 2
        
        # Can use test_clock directly
        assert test_clock.current_time_ms() == 200.0  # 2 steps * 100ms
        
        # Switch back to production
        ClockFactory.set_test_mode(False)
        assert isinstance(ClockFactory.create(), WallClock)
