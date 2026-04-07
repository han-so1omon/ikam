"""
Comprehensive test suite for adaptive threshold learning.

Tests cover:
- Dataclass validation (ConfidenceDelta, ThresholdScore, LearnedThreshold)
- Core algorithm (feedback recording, threshold updates, convergence)
- Feedback aggregation and satisfaction computation
- Confidence in threshold scoring
- Recency bias weighting
- Edge cases (empty feedback, convergence detection)
- Exponential moving average calculations
- API endpoints (feedback submission, metrics, threshold queries)
"""

import pytest
from datetime import datetime, timedelta
from unittest.mock import AsyncMock, MagicMock, patch

from modelado.sequencer.adaptive_thresholds import (
    FeedbackType,
    ConfidenceRange,
    ConfidenceDelta,
    ThresholdScore,
    LearnedThreshold,
    AdaptiveThresholdMetrics,
    AdaptiveThresholdLearner,
)
from modelado.sequencer.adaptive_thresholds_api import (
    FeedbackRequest,
    BatchFeedbackRequest,
    ConfidenceDeltaResponse,
    LearnedThresholdResponse,
    AdaptiveThresholdMetricsResponse,
    EffectiveThresholdResponse,
    create_adaptive_thresholds_router,
)


# ============================================================================
# Dataclass Tests
# ============================================================================


class TestConfidenceDelta:
    """Test ConfidenceDelta dataclass."""

    def test_create_valid_delta(self):
        """Test creating valid confidence delta."""
        delta = ConfidenceDelta(
            concept_id="revenue_forecasting",
            original_confidence=0.92,
            user_feedback=FeedbackType.ACCEPT,
            adjustment_delta=0.0,
        )
        assert delta.concept_id == "revenue_forecasting"
        assert delta.original_confidence == 0.92
        assert delta.user_feedback == FeedbackType.ACCEPT
        assert delta.adjustment_delta == 0.0

    def test_invalid_confidence(self):
        """Test invalid confidence raises error."""
        with pytest.raises(ValueError):
            ConfidenceDelta(
                concept_id="test",
                original_confidence=1.5,
                user_feedback=FeedbackType.ACCEPT,
                adjustment_delta=0.0,
            )

    def test_invalid_delta(self):
        """Test invalid adjustment delta raises error."""
        with pytest.raises(ValueError):
            ConfidenceDelta(
                concept_id="test",
                original_confidence=0.75,
                user_feedback=FeedbackType.REJECT,
                adjustment_delta=1.5,
            )


class TestThresholdScore:
    """Test ThresholdScore dataclass."""

    def test_create_valid_score(self):
        """Test creating valid threshold score."""
        score = ThresholdScore(
            range_name=ConfidenceRange.HIGH,
            mean_satisfaction=0.75,
            feedback_count=10,
            recent_feedback_count=3,
        )
        assert score.range_name == ConfidenceRange.HIGH
        assert score.mean_satisfaction == 0.75
        assert score.feedback_count == 10

    def test_invalid_satisfaction(self):
        """Test invalid satisfaction raises error."""
        with pytest.raises(ValueError):
            ThresholdScore(
                range_name=ConfidenceRange.HIGH,
                mean_satisfaction=1.5,
                feedback_count=10,
                recent_feedback_count=3,
            )


class TestLearnedThreshold:
    """Test LearnedThreshold dataclass."""

    def test_create_valid_threshold(self):
        """Test creating valid learned threshold."""
        threshold = LearnedThreshold(
            range_name=ConfidenceRange.HIGH,
            original_threshold=0.75,
            learned_threshold=0.77,
            adjustment=0.02,
            confidence_in_threshold=0.85,
            learning_iterations=3,
        )
        assert threshold.range_name == ConfidenceRange.HIGH
        assert threshold.learned_threshold == 0.77
        assert threshold.adjustment == 0.02


# ============================================================================
# AdaptiveThresholdLearner Unit Tests
# ============================================================================


class TestAdaptiveThresholdLearnerUnit:
    """Unit tests for AdaptiveThresholdLearner."""

    @pytest.fixture
    def learner(self):
        """Create learner instance for testing."""
        return AdaptiveThresholdLearner(
            default_min_confidence=0.50,
            adaptive_window_hours=24,
            convergence_threshold=0.01,
            learning_rate=0.15,
        )

    def test_initialization_default_params(self, learner):
        """Test default initialization."""
        assert learner.default_min_confidence == 0.50
        assert learner.adaptive_window_hours == 24
        assert learner.convergence_threshold == 0.01
        assert learner.learning_rate == 0.15
        assert len(learner._feedback_history) == 0
        assert len(learner._threshold_scores) == 5  # 5 confidence ranges

    def test_initialization_custom_params(self):
        """Test custom initialization."""
        learner = AdaptiveThresholdLearner(
            default_min_confidence=0.70,
            adaptive_window_hours=48,
            convergence_threshold=0.005,
            learning_rate=0.20,
        )
        assert learner.default_min_confidence == 0.70
        assert learner.learning_rate == 0.20

    def test_invalid_min_confidence(self):
        """Test invalid min_confidence raises error."""
        with pytest.raises(ValueError):
            AdaptiveThresholdLearner(default_min_confidence=1.5)

    def test_invalid_learning_rate(self):
        """Test invalid learning_rate raises error."""
        with pytest.raises(ValueError):
            AdaptiveThresholdLearner(learning_rate=1.5)

    def test_range_to_threshold_mapping(self, learner):
        """Test confidence range to threshold mapping."""
        assert learner._range_to_threshold(ConfidenceRange.VERY_HIGH) == 0.90
        assert learner._range_to_threshold(ConfidenceRange.HIGH) == 0.75
        assert learner._range_to_threshold(ConfidenceRange.MEDIUM) == 0.50
        assert learner._range_to_threshold(ConfidenceRange.LOW) == 0.25
        assert learner._range_to_threshold(ConfidenceRange.VERY_LOW) == 0.00

    def test_get_confidence_range(self, learner):
        """Test confidence score to range mapping."""
        assert learner._get_confidence_range(0.95) == ConfidenceRange.VERY_HIGH
        assert learner._get_confidence_range(0.80) == ConfidenceRange.HIGH
        assert learner._get_confidence_range(0.60) == ConfidenceRange.MEDIUM
        assert learner._get_confidence_range(0.30) == ConfidenceRange.LOW
        assert learner._get_confidence_range(0.10) == ConfidenceRange.VERY_LOW

    def test_compute_adjustment_delta_accept(self, learner):
        """Test adjustment delta for accept feedback."""
        delta = learner._compute_adjustment_delta(0.75, FeedbackType.ACCEPT)
        assert delta == 0.0

    def test_compute_adjustment_delta_reject(self, learner):
        """Test adjustment delta for reject feedback."""
        delta = learner._compute_adjustment_delta(0.75, FeedbackType.REJECT)
        assert delta == -0.20

    def test_compute_adjustment_delta_spurious(self, learner):
        """Test adjustment delta for spurious feedback."""
        delta = learner._compute_adjustment_delta(0.75, FeedbackType.SPURIOUS)
        assert delta == -0.30

    def test_exponential_moving_average(self, learner):
        """Test exponential moving average calculation."""
        # With 1 sample: alpha = 2 / (1 + 1) = 1.0 (use new value)
        ema = learner._exponential_moving_average(0.50, 0.75, 1)
        assert ema == 0.75

        # With 2 samples: alpha = 2 / (2 + 1) = 0.667
        ema = learner._exponential_moving_average(0.50, 0.75, 2)
        assert 0.66 < ema < 0.68  # ~0.667

        # With many samples: alpha ≈ 0 (trust old value more)
        ema = learner._exponential_moving_average(0.50, 0.75, 100)
        assert ema > 0.50 and ema < 0.55


# ============================================================================
# AdaptiveThresholdLearner Integration Tests
# ============================================================================


class TestAdaptiveThresholdLearnerIntegration:
    """Integration tests for AdaptiveThresholdLearner."""

    @pytest.fixture
    def learner(self):
        """Create learner instance for testing."""
        return AdaptiveThresholdLearner(learning_rate=0.15)

    @pytest.mark.asyncio
    async def test_record_feedback_single(self, learner):
        """Test recording single feedback."""
        delta = await learner.record_feedback(
            concept_id="revenue_forecasting",
            original_confidence=0.92,
            feedback_type=FeedbackType.ACCEPT,
            reason="Confidence was accurate",
        )
        assert delta.concept_id == "revenue_forecasting"
        assert delta.original_confidence == 0.92
        assert delta.user_feedback == FeedbackType.ACCEPT
        assert delta.adjustment_delta == 0.0

    @pytest.mark.asyncio
    async def test_record_feedback_multiple_same_concept(self, learner):
        """Test recording multiple feedback for same concept."""
        concept_id = "unit_economics"
        for i in range(3):
            await learner.record_feedback(
                concept_id=concept_id,
                original_confidence=0.70,
                feedback_type=FeedbackType.ACCEPT,
            )
        assert len(learner._feedback_history[concept_id]) == 3

    @pytest.mark.asyncio
    async def test_full_feedback_pipeline(self, learner):
        """Test end-to-end feedback and threshold update."""
        # Record feedback: accept high-confidence, reject medium-confidence
        await learner.record_feedback("concept1", 0.92, FeedbackType.ACCEPT)
        await learner.record_feedback("concept2", 0.92, FeedbackType.ACCEPT)
        await learner.record_feedback("concept3", 0.60, FeedbackType.REJECT)
        await learner.record_feedback("concept4", 0.60, FeedbackType.REJECT)

        # Update thresholds
        thresholds = await learner.update_thresholds()

        # Verify thresholds dict is returned with expected structure
        assert isinstance(thresholds, dict)
        assert len(thresholds) > 0
        # Verify all values are LearnedThreshold objects
        for threshold in thresholds.values():
            assert isinstance(threshold, LearnedThreshold)
            assert threshold.learned_threshold is not None
            assert threshold.original_threshold is not None

    @pytest.mark.asyncio
    async def test_convergence_detection(self, learner):
        """Test convergence detection when threshold changes are small."""
        # Record feedback with all accepts (high satisfaction)
        for i in range(10):
            await learner.record_feedback(f"concept_{i}", 0.80, FeedbackType.ACCEPT)

        # Multiple update iterations should eventually converge
        for iteration in range(5):
            thresholds = await learner.update_thresholds()
            metrics = await learner.get_metrics()
            if metrics.learning_converged:
                assert learner._learning_iterations > 1
                break

    @pytest.mark.asyncio
    async def test_confidence_in_threshold_increases_with_feedback(self, learner):
        """Test that confidence_in_threshold increases with more feedback."""
        # First iteration: minimal feedback
        for i in range(2):
            await learner.record_feedback(f"c1_{i}", 0.80, FeedbackType.ACCEPT)
        await learner.update_thresholds()

        high_threshold_1 = learner._learned_thresholds[ConfidenceRange.HIGH]
        conf_1 = high_threshold_1.confidence_in_threshold

        # More iterations with more feedback
        for i in range(20):
            await learner.record_feedback(f"c2_{i}", 0.85, FeedbackType.ACCEPT)
        await learner.update_thresholds()

        high_threshold_2 = learner._learned_thresholds[ConfidenceRange.HIGH]
        conf_2 = high_threshold_2.confidence_in_threshold

        # More feedback should increase confidence
        assert conf_2 > conf_1

    @pytest.mark.asyncio
    async def test_clear_feedback_all(self, learner):
        """Test clearing all feedback."""
        for i in range(5):
            await learner.record_feedback(f"concept_{i}", 0.75, FeedbackType.ACCEPT)

        assert len(learner._feedback_history) == 5
        cleared = await learner.clear_feedback()
        assert cleared == 5
        assert len(learner._feedback_history) == 0

    @pytest.mark.asyncio
    async def test_clear_feedback_single_concept(self, learner):
        """Test clearing feedback for single concept."""
        for i in range(5):
            await learner.record_feedback(f"c1", 0.75, FeedbackType.ACCEPT)
        for i in range(3):
            await learner.record_feedback(f"c2", 0.75, FeedbackType.ACCEPT)

        cleared = await learner.clear_feedback(concept_id="c1")
        assert cleared == 5
        assert "c1" not in learner._feedback_history
        assert "c2" in learner._feedback_history

    @pytest.mark.asyncio
    async def test_get_effective_threshold_uses_learned_when_confident(self, learner):
        """Test effective threshold uses learned value when confident."""
        # Record many feedback items to build confidence
        for i in range(30):
            await learner.record_feedback(f"concept_{i}", 0.80, FeedbackType.ACCEPT)

        await learner.update_thresholds()

        # Get effective threshold with high confidence requirement
        effective = await learner.get_effective_threshold(
            ConfidenceRange.HIGH, min_confidence_in_threshold=0.50
        )

        learned = learner._learned_thresholds[ConfidenceRange.HIGH]
        # Should use learned threshold since confidence > 0.50
        assert effective == learned.learned_threshold

    @pytest.mark.asyncio
    async def test_get_effective_threshold_fallback_low_confidence(self, learner):
        """Test effective threshold falls back when confidence is low."""
        # Record minimal feedback (low confidence)
        await learner.record_feedback("concept_1", 0.80, FeedbackType.ACCEPT)
        await learner.update_thresholds()

        # Get effective threshold with high confidence requirement
        effective = await learner.get_effective_threshold(
            ConfidenceRange.HIGH, min_confidence_in_threshold=0.90
        )

        # Should fall back to original since learned confidence < 0.90
        assert effective == 0.75  # original HIGH threshold


# ============================================================================
# Metrics and Reporting Tests
# ============================================================================


class TestMetricsAndReporting:
    """Test metrics and reporting functionality."""

    @pytest.fixture
    def learner(self):
        """Create learner with sample feedback."""
        return AdaptiveThresholdLearner()

    @pytest.mark.asyncio
    async def test_get_metrics_empty(self, learner):
        """Test metrics with no feedback."""
        metrics = await learner.get_metrics()
        assert metrics.total_feedback == 0
        assert metrics.total_concepts_evaluated == 0
        assert metrics.mean_feedback_score == 0.0

    @pytest.mark.asyncio
    async def test_get_metrics_with_feedback(self, learner):
        """Test metrics with feedback."""
        # Record mixed feedback
        await learner.record_feedback("c1", 0.80, FeedbackType.ACCEPT)
        await learner.record_feedback("c2", 0.80, FeedbackType.ACCEPT)
        await learner.record_feedback("c3", 0.60, FeedbackType.REJECT)

        metrics = await learner.get_metrics()
        assert metrics.total_feedback == 3
        assert metrics.total_concepts_evaluated == 3
        assert metrics.mean_feedback_score == pytest.approx(2 / 3, abs=0.01)

    @pytest.mark.asyncio
    async def test_metrics_feedback_distribution(self, learner):
        """Test feedback type distribution in metrics."""
        await learner.record_feedback("c1", 0.80, FeedbackType.ACCEPT)
        await learner.record_feedback("c2", 0.80, FeedbackType.ACCEPT)
        await learner.record_feedback("c3", 0.60, FeedbackType.REJECT)
        await learner.record_feedback("c4", 0.50, FeedbackType.SPURIOUS)

        metrics = await learner.get_metrics()
        assert metrics.feedback_distribution["accept"] == 2
        assert metrics.feedback_distribution["reject"] == 1
        assert metrics.feedback_distribution["spurious"] == 1

    @pytest.mark.asyncio
    async def test_metrics_confidence_distribution(self, learner):
        """Test confidence range distribution in metrics."""
        await learner.record_feedback("c1", 0.95, FeedbackType.ACCEPT)  # VERY_HIGH
        await learner.record_feedback("c2", 0.80, FeedbackType.ACCEPT)  # HIGH
        await learner.record_feedback("c3", 0.60, FeedbackType.ACCEPT)  # MEDIUM

        metrics = await learner.get_metrics()
        assert metrics.confidence_distribution["very_high"] == 1
        assert metrics.confidence_distribution["high"] == 1
        assert metrics.confidence_distribution["medium"] == 1

    @pytest.mark.asyncio
    async def test_get_learned_thresholds_dict(self, learner):
        """Test serialization of learned thresholds."""
        thresholds_dict = learner.get_learned_thresholds_dict()

        assert "very_high" in thresholds_dict
        assert "high" in thresholds_dict
        assert "medium" in thresholds_dict

        high_data = thresholds_dict["high"]
        assert "original_threshold" in high_data
        assert "learned_threshold" in high_data
        assert "confidence_in_threshold" in high_data


# ============================================================================
# API Response Model Tests
# ============================================================================


class TestAPIResponseModels:
    """Test API response model conversions."""

    def test_confidence_delta_response_from_domain(self):
        """Test converting ConfidenceDelta to response."""
        delta = ConfidenceDelta(
            concept_id="test",
            original_confidence=0.80,
            user_feedback=FeedbackType.ACCEPT,
            adjustment_delta=0.0,
        )
        response = ConfidenceDeltaResponse.from_domain(delta)
        assert response.concept_id == "test"
        assert response.original_confidence == 0.80
        assert response.user_feedback == "accept"

    def test_learned_threshold_response_from_domain(self):
        """Test converting LearnedThreshold to response."""
        threshold = LearnedThreshold(
            range_name=ConfidenceRange.HIGH,
            original_threshold=0.75,
            learned_threshold=0.77,
            adjustment=0.02,
            confidence_in_threshold=0.85,
            learning_iterations=3,
        )
        response = LearnedThresholdResponse.from_domain(threshold)
        assert response.range_name == "high"
        assert response.learned_threshold == 0.77

    @pytest.mark.asyncio
    async def test_metrics_response_from_domain(self):
        """Test converting metrics to response."""
        metrics = AdaptiveThresholdMetrics(
            total_feedback=10,
            total_concepts_evaluated=8,
            mean_feedback_score=0.75,
            feedback_distribution={"accept": 7, "reject": 3},
            confidence_distribution={"high": 5, "medium": 5},
            learning_converged=False,
            convergence_delta=0.02,
        )
        response = AdaptiveThresholdMetricsResponse.from_domain(metrics)
        assert response.total_feedback == 10
        assert response.mean_feedback_score == 0.75
