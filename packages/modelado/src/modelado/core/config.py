"""Configuration management for generative operations and traversal system.

Provides externalized configuration for traversal timing, spike/decay models,
and testing-friendly pluggable clock injection.
"""

from dataclasses import dataclass
from typing import Optional
import os


@dataclass(frozen=True)
class TraversalConfig:
    """Configuration for traversal execution.
    
    Environment Variables:
        TRAVERSAL_ENABLE_SPIKE_DECAY: Enable spike/decay timing (default: true)
        TRAVERSAL_BASE_STEP_DURATION_MS: Base step duration in milliseconds (default: 100)
        TRAVERSAL_SPIKE_MULTIPLIER: Spike duration multiplier (default: 2.0)
        TRAVERSAL_DECAY_HALF_LIFE_STEPS: Decay half-life in steps (default: 10)
        TRAVERSAL_MIN_STEP_DURATION_MS: Minimum step duration (default: 10)
        TRAVERSAL_MAX_STEP_DURATION_MS: Maximum step duration (default: 5000)
    """
    
    enable_spike_decay: bool = True
    base_step_duration_ms: int = 100
    spike_multiplier: float = 2.0
    decay_half_life_steps: int = 10
    min_step_duration_ms: int = 10
    max_step_duration_ms: int = 5000
    
    @classmethod
    def from_env(cls) -> "TraversalConfig":
        """Load configuration from environment variables."""
        return cls(
            enable_spike_decay=os.getenv("TRAVERSAL_ENABLE_SPIKE_DECAY", "true").lower() == "true",
            base_step_duration_ms=int(os.getenv("TRAVERSAL_BASE_STEP_DURATION_MS", "100")),
            spike_multiplier=float(os.getenv("TRAVERSAL_SPIKE_MULTIPLIER", "2.0")),
            decay_half_life_steps=int(os.getenv("TRAVERSAL_DECAY_HALF_LIFE_STEPS", "10")),
            min_step_duration_ms=int(os.getenv("TRAVERSAL_MIN_STEP_DURATION_MS", "10")),
            max_step_duration_ms=int(os.getenv("TRAVERSAL_MAX_STEP_DURATION_MS", "5000")),
        )
    
    def validate(self) -> None:
        """Validate configuration constraints.
        
        Raises:
            ValueError: If any constraint is violated
        """
        if self.base_step_duration_ms <= 0:
            raise ValueError("base_step_duration_ms must be > 0")
        if self.spike_multiplier < 1.0:
            raise ValueError("spike_multiplier must be >= 1.0")
        if self.decay_half_life_steps <= 0:
            raise ValueError("decay_half_life_steps must be > 0")
        if self.min_step_duration_ms <= 0:
            raise ValueError("min_step_duration_ms must be > 0")
        if self.max_step_duration_ms < self.min_step_duration_ms:
            raise ValueError("max_step_duration_ms must be >= min_step_duration_ms")


@dataclass(frozen=True)
class TimingConfig:
    """Configuration for operation timing and metrics.
    
    Environment Variables:
        TIMING_ENABLE_METRICS: Enable timing metrics (default: true)
        TIMING_SAMPLE_RATE: Sampling rate for metrics (0.0-1.0, default: 1.0)
        TIMING_BUCKET_COUNT: Number of histogram buckets (default: 10)
    """
    
    enable_metrics: bool = True
    sample_rate: float = 1.0
    bucket_count: int = 10
    
    @classmethod
    def from_env(cls) -> "TimingConfig":
        """Load configuration from environment variables."""
        return cls(
            enable_metrics=os.getenv("TIMING_ENABLE_METRICS", "true").lower() == "true",
            sample_rate=float(os.getenv("TIMING_SAMPLE_RATE", "1.0")),
            bucket_count=int(os.getenv("TIMING_BUCKET_COUNT", "10")),
        )
    
    def validate(self) -> None:
        """Validate configuration constraints."""
        if not (0.0 <= self.sample_rate <= 1.0):
            raise ValueError("sample_rate must be between 0.0 and 1.0")
        if self.bucket_count <= 0:
            raise ValueError("bucket_count must be > 0")


class StepClock:
    """Test-friendly clock for step-based timing.
    
    Allows deterministic control of step counts and durations for testing.
    In production, use WallClock.
    """
    
    def __init__(self, initial_step: int = 0):
        """Initialize step clock.
        
        Args:
            initial_step: Starting step count
        """
        self.current_step = initial_step
        self.step_duration_ms = 100
    
    def tick(self) -> int:
        """Advance to next step.
        
        Returns:
            Current step number
        """
        self.current_step += 1
        return self.current_step
    
    def current_time_ms(self) -> float:
        """Get current time in milliseconds.
        
        Returns:
            Cumulative duration based on steps * step_duration
        """
        return self.current_step * self.step_duration_ms
    
    def advance_ms(self, duration_ms: float) -> None:
        """Advance clock by specific duration (testing only).
        
        Args:
            duration_ms: Duration to advance in milliseconds
        """
        steps = max(1, int(duration_ms / self.step_duration_ms))
        self.current_step += steps
    
    def reset(self) -> None:
        """Reset clock to initial state."""
        self.current_step = 0


class WallClock:
    """Production clock using real wall-clock time.
    
    Provides accurate timing for performance metrics and spike/decay calculations.
    """
    
    import time
    
    def __init__(self):
        """Initialize wall clock."""
        self.start_time_ns = self.time.time_ns()
        self._last_tick_ns = self.start_time_ns
    
    def tick(self) -> int:
        """Get elapsed time since creation.
        
        Returns:
            Elapsed time in milliseconds
        """
        self._last_tick_ns = self.time.time_ns()
        return int((self._last_tick_ns - self.start_time_ns) / 1_000_000)
    
    def current_time_ms(self) -> float:
        """Get current elapsed time in milliseconds.
        
        Returns:
            Elapsed time in milliseconds since creation
        """
        current_ns = self.time.time_ns()
        return (current_ns - self.start_time_ns) / 1_000_000
    
    def reset(self) -> None:
        """Reset clock to current time."""
        self.start_time_ns = self.time.time_ns()
        self._last_tick_ns = self.start_time_ns


class ClockFactory:
    """Factory for creating appropriate clock instances.
    
    In production: creates WallClock
    In tests: creates StepClock for deterministic control
    """
    
    _test_mode = False
    
    @classmethod
    def set_test_mode(cls, enabled: bool) -> None:
        """Enable/disable test mode for clock creation.
        
        Args:
            enabled: True to use StepClock, False to use WallClock
        """
        cls._test_mode = enabled
    
    @classmethod
    def create(cls) -> "StepClock | WallClock":
        """Create appropriate clock instance.
        
        Returns:
            StepClock in test mode, WallClock otherwise
        """
        if cls._test_mode:
            return StepClock()
        return WallClock()


# Global configuration instances
_traversal_config: Optional[TraversalConfig] = None
_timing_config: Optional[TimingConfig] = None


def get_traversal_config() -> TraversalConfig:
    """Get global traversal configuration.
    
    Loads from environment on first call, caches result.
    
    Returns:
        TraversalConfig instance
    """
    global _traversal_config
    if _traversal_config is None:
        _traversal_config = TraversalConfig.from_env()
        _traversal_config.validate()
    return _traversal_config


def get_timing_config() -> TimingConfig:
    """Get global timing configuration.
    
    Loads from environment on first call, caches result.
    
    Returns:
        TimingConfig instance
    """
    global _timing_config
    if _timing_config is None:
        _timing_config = TimingConfig.from_env()
        _timing_config.validate()
    return _timing_config


def reset_config() -> None:
    """Reset cached configuration (testing only).
    
    Forces reload from environment on next access.
    """
    global _traversal_config, _timing_config
    _traversal_config = None
    _timing_config = None
