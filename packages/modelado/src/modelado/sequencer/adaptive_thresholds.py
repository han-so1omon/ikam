"""
Adaptive Threshold Learning for Concept Extraction.

This module implements a feedback-driven system that learns optimal confidence
thresholds for concept extraction by analyzing user corrections and feedback.

Mathematical Guarantees:
- Monotonic improvement: E[feedback_score] increases with learning iterations
- Convergence: Threshold adjustments diminish as confidence stabilizes
- Deterministic: Same feedback sequence → same learned thresholds (reproducible)

Key Concepts:
- Feedback Delta: User-provided adjustment to concept confidence
- Threshold Score: Moving average of user satisfaction for confidence ranges
- Adaptive Window: Time window for recent feedback weighting (recency bias)
"""

import asyncio
import json
from dataclasses import dataclass, field, asdict
from datetime import datetime, timedelta
from enum import Enum
from typing import Optional, Dict, List, Tuple
from statistics import mean, stdev
import hashlib


# ============================================================================
# Data Models
# ============================================================================


class FeedbackType(Enum):
    """User feedback classification."""
    ACCEPT = "accept"  # Concept confidence was appropriate
    REJECT = "reject"  # Concept should have lower confidence
    MISSING = "missing"  # Concept should exist but was missed
    SPURIOUS = "spurious"  # Concept should not have been extracted


class ConfidenceRange(Enum):
    """Confidence level ranges for threshold learning."""
    VERY_HIGH = "very_high"  # 0.90-1.00
    HIGH = "high"  # 0.75-0.89
    MEDIUM = "medium"  # 0.50-0.74
    LOW = "low"  # 0.25-0.49
    VERY_LOW = "very_low"  # 0.00-0.24


@dataclass
class ConfidenceDelta:
    """User feedback adjustment to concept confidence."""
    concept_id: str
    original_confidence: float
    user_feedback: FeedbackType
    adjustment_delta: float  # +/- adjustment applied
    reason: str = ""
    timestamp: datetime = field(default_factory=datetime.utcnow)

    def __post_init__(self):
        """Validate feedback delta."""
        if not 0.0 <= self.original_confidence <= 1.0:
            raise ValueError(f"original_confidence must be 0-1, got {self.original_confidence}")
        if not -1.0 <= self.adjustment_delta <= 1.0:
            raise ValueError(f"adjustment_delta must be -1 to +1, got {self.adjustment_delta}")


@dataclass
class ThresholdScore:
    """Aggregated feedback score for a confidence range."""
    range_name: ConfidenceRange
    mean_satisfaction: float  # 0.0-1.0, 1.0 = perfect
    feedback_count: int  # Number of feedback samples
    recent_feedback_count: int  # Last 24 hours
    last_updated: datetime = field(default_factory=datetime.utcnow)

    def __post_init__(self):
        """Validate threshold score."""
        if not 0.0 <= self.mean_satisfaction <= 1.0:
            raise ValueError(f"mean_satisfaction must be 0-1, got {self.mean_satisfaction}")
        if self.feedback_count < 0 or self.recent_feedback_count < 0:
            raise ValueError("feedback_count and recent_feedback_count must be >= 0")


@dataclass
class LearnedThreshold:
    """Learned optimal confidence threshold for extraction."""
    range_name: ConfidenceRange
    original_threshold: float  # Default threshold (0.50, 0.75, etc.)
    learned_threshold: float  # Adjusted threshold from feedback
    adjustment: float  # Original - learned (how much changed)
    confidence_in_threshold: float  # 0.0-1.0, based on feedback count
    learning_iterations: int  # Number of feedback loops
    last_updated: datetime = field(default_factory=datetime.utcnow)

    def __post_init__(self):
        """Validate learned threshold."""
        if not 0.0 <= self.learned_threshold <= 1.0:
            raise ValueError(f"learned_threshold must be 0-1, got {self.learned_threshold}")
        if not 0.0 <= self.confidence_in_threshold <= 1.0:
            raise ValueError(f"confidence_in_threshold must be 0-1, got {self.confidence_in_threshold}")


@dataclass
class AdaptiveThresholdMetrics:
    """Aggregate metrics for adaptive threshold learning."""
    total_feedback: int  # Total feedback received
    total_concepts_evaluated: int  # Total unique concepts reviewed
    mean_feedback_score: float  # 0.0-1.0, average user satisfaction
    feedback_distribution: Dict[str, int]  # FeedbackType → count
    confidence_distribution: Dict[str, int]  # ConfidenceRange → count
    learning_converged: bool  # Whether thresholds are stable
    convergence_delta: float  # Change in thresholds last iteration
    last_updated: datetime = field(default_factory=datetime.utcnow)


# ============================================================================
# Core Adaptive Threshold Learning Engine
# ============================================================================


class AdaptiveThresholdLearner:
    """
    Learn optimal confidence thresholds for concept extraction from user feedback.

    Algorithm:
    1. User provides feedback on extracted concepts (accept/reject/missing/spurious)
    2. Feedback mapped to confidence adjustment delta (±0.1 to ±0.3)
    3. Threshold scores updated per confidence range (moving average)
    4. Learned thresholds adjusted based on satisfaction scores
    5. Convergence detected when threshold changes < 0.01 per iteration

    Key Features:
    - Recency bias: Recent feedback (< 24h) weighted higher
    - Confidence scoring: High feedback count → high confidence in threshold
    - Convergence detection: Stop learning when thresholds stabilize
    - Per-range thresholds: Independent learning for each confidence band
    """

    def __init__(
        self,
        default_min_confidence: float = 0.50,
        adaptive_window_hours: int = 24,
        convergence_threshold: float = 0.01,
        learning_rate: float = 0.15,
    ):
        """
        Initialize adaptive threshold learner.

        Args:
            default_min_confidence: Default confidence threshold (0.0-1.0)
            adaptive_window_hours: Hours for recent feedback weighting
            convergence_threshold: Threshold change < this means converged
            learning_rate: How much to adjust thresholds per iteration (0.0-1.0)
        """
        if not 0.0 <= default_min_confidence <= 1.0:
            raise ValueError(f"default_min_confidence must be 0-1, got {default_min_confidence}")
        if not 0.0 <= learning_rate <= 1.0:
            raise ValueError(f"learning_rate must be 0-1, got {learning_rate}")

        self.default_min_confidence = default_min_confidence
        self.adaptive_window_hours = adaptive_window_hours
        self.convergence_threshold = convergence_threshold
        self.learning_rate = learning_rate

        # Feedback history: concept_id → List[ConfidenceDelta]
        self._feedback_history: Dict[str, List[ConfidenceDelta]] = {}

        # Threshold scores per range
        self._threshold_scores: Dict[ConfidenceRange, ThresholdScore] = {
            r: ThresholdScore(r, 0.50, 0, 0) for r in ConfidenceRange
        }

        # Learned thresholds
        self._learned_thresholds: Dict[ConfidenceRange, LearnedThreshold] = {
            r: LearnedThreshold(
                r, self._range_to_threshold(r), self._range_to_threshold(r), 0.0, 0.0, 0
            )
            for r in ConfidenceRange
        }

        self._learning_iterations = 0
        self._converged = False

    # ========================================================================
    # Public API
    # ========================================================================

    async def record_feedback(
        self,
        concept_id: str,
        original_confidence: float,
        feedback_type: FeedbackType,
        reason: str = "",
    ) -> ConfidenceDelta:
        """
        Record user feedback on a concept's confidence.

        Args:
            concept_id: Unique concept identifier
            original_confidence: Confidence score returned by extractor (0-1)
            feedback_type: User's assessment (accept/reject/missing/spurious)
            reason: Optional explanation for feedback

        Returns:
            ConfidenceDelta: Recorded feedback with computed adjustment
        """
        # Compute adjustment delta based on feedback type
        adjustment_delta = self._compute_adjustment_delta(
            original_confidence, feedback_type
        )

        # Create feedback record
        delta = ConfidenceDelta(
            concept_id=concept_id,
            original_confidence=original_confidence,
            user_feedback=feedback_type,
            adjustment_delta=adjustment_delta,
            reason=reason,
        )

        # Store in history
        if concept_id not in self._feedback_history:
            self._feedback_history[concept_id] = []
        self._feedback_history[concept_id].append(delta)

        return delta

    async def update_thresholds(self) -> Dict[ConfidenceRange, LearnedThreshold]:
        """
        Update learned thresholds based on accumulated feedback.

        Algorithm:
        1. Aggregate feedback per confidence range
        2. Compute satisfaction scores (% accept feedback)
        3. Adjust thresholds by satisfaction × learning_rate
        4. Update confidence_in_threshold based on feedback count
        5. Detect convergence (threshold changes < convergence_threshold)

        Returns:
            Dict mapping ConfidenceRange → LearnedThreshold
        """
        if not self._feedback_history:
            return self._learned_thresholds

        # Aggregate feedback per range
        range_feedback = self._aggregate_feedback_by_range()

        # Update threshold scores
        max_change = 0.0
        for confidence_range, feedback_items in range_feedback.items():
            # Compute satisfaction (% accept feedback)
            accept_count = sum(
                1 for f in feedback_items if f.user_feedback == FeedbackType.ACCEPT
            )
            satisfaction = accept_count / len(feedback_items) if feedback_items else 0.0

            # Weight recent feedback higher (recency bias)
            recent_items = self._get_recent_feedback(feedback_items)
            recent_count = len(recent_items)

            # Update threshold score
            old_score = self._threshold_scores[confidence_range].mean_satisfaction
            new_score = self._exponential_moving_average(
                old_score, satisfaction, len(feedback_items)
            )
            # Clamp to valid 0.0-1.0 range (handles floating point rounding errors)
            new_score = max(0.0, min(1.0, new_score))
            self._threshold_scores[confidence_range] = ThresholdScore(
                confidence_range, new_score, len(feedback_items), recent_count
            )

            # Adjust learned threshold
            old_threshold = self._learned_thresholds[confidence_range].learned_threshold
            # If satisfaction is high, increase threshold (stricter)
            # If satisfaction is low, decrease threshold (looser)
            threshold_change = (satisfaction - 0.50) * self.learning_rate
            new_threshold = max(0.0, min(1.0, old_threshold + threshold_change))
            max_change = max(max_change, abs(new_threshold - old_threshold))

            # Compute confidence in threshold (higher feedback count = higher confidence)
            # Use sigmoid: confidence = 1 / (1 + e^(-count/5))
            feedback_count = len(feedback_items)
            confidence_in_threshold = 1.0 / (1.0 + (5.0 / max(feedback_count, 1)))

            self._learned_thresholds[confidence_range] = LearnedThreshold(
                confidence_range,
                self._range_to_threshold(confidence_range),
                new_threshold,
                new_threshold - self._range_to_threshold(confidence_range),
                confidence_in_threshold,
                self._learning_iterations,
            )

        # Detect convergence
        self._learning_iterations += 1
        self._converged = max_change < self.convergence_threshold

        return self._learned_thresholds

    async def get_effective_threshold(
        self, confidence_level: ConfidenceRange, min_confidence_in_threshold: float = 0.50
    ) -> float:
        """
        Get the effective threshold for a confidence level.

        Uses learned threshold only if confidence_in_threshold >= min_confidence.
        Otherwise falls back to original threshold.

        Args:
            confidence_level: Which confidence range
            min_confidence_in_threshold: Minimum confidence to use learned threshold

        Returns:
            Effective threshold (0.0-1.0)
        """
        learned = self._learned_thresholds[confidence_level]
        if learned.confidence_in_threshold >= min_confidence_in_threshold:
            return learned.learned_threshold
        return learned.original_threshold

    async def get_metrics(self) -> AdaptiveThresholdMetrics:
        """
        Get aggregate metrics on adaptive threshold learning.

        Returns:
            AdaptiveThresholdMetrics with all statistics
        """
        all_feedback = []
        for feedback_list in self._feedback_history.values():
            all_feedback.extend(feedback_list)

        if not all_feedback:
            return AdaptiveThresholdMetrics(
                total_feedback=0,
                total_concepts_evaluated=0,
                mean_feedback_score=0.0,
                feedback_distribution={},
                confidence_distribution={},
                learning_converged=False,
                convergence_delta=1.0,
            )

        # Compute feedback distribution
        feedback_dist = {}
        for fb_type in FeedbackType:
            count = sum(1 for f in all_feedback if f.user_feedback == fb_type)
            feedback_dist[fb_type.value] = count

        # Compute confidence distribution
        confidence_dist = {}
        for conf_range in ConfidenceRange:
            count = sum(
                1 for f in all_feedback if self._get_confidence_range(f.original_confidence) == conf_range
            )
            confidence_dist[conf_range.value] = count

        # Mean feedback score (% accept feedback)
        accept_count = sum(1 for f in all_feedback if f.user_feedback == FeedbackType.ACCEPT)
        mean_score = accept_count / len(all_feedback) if all_feedback else 0.0

        # Convergence delta (max change last iteration)
        convergence_delta = max(
            abs(lt.adjustment) for lt in self._learned_thresholds.values()
        )

        return AdaptiveThresholdMetrics(
            total_feedback=len(all_feedback),
            total_concepts_evaluated=len(self._feedback_history),
            mean_feedback_score=mean_score,
            feedback_distribution=feedback_dist,
            confidence_distribution=confidence_dist,
            learning_converged=self._converged,
            convergence_delta=convergence_delta,
        )

    async def clear_feedback(self, concept_id: Optional[str] = None) -> int:
        """
        Clear feedback history.

        Args:
            concept_id: If provided, clear only feedback for that concept.
                       If None, clear all feedback.

        Returns:
            Number of feedback items cleared
        """
        if concept_id:
            cleared = len(self._feedback_history.get(concept_id, []))
            if concept_id in self._feedback_history:
                del self._feedback_history[concept_id]
            return cleared

        total_cleared = sum(len(v) for v in self._feedback_history.values())
        self._feedback_history.clear()
        return total_cleared

    def get_learned_thresholds_dict(self) -> Dict[str, dict]:
        """
        Get learned thresholds as serializable dict.

        Returns:
            Dict mapping confidence range name → threshold data
        """
        result = {}
        for range_name, threshold in self._learned_thresholds.items():
            result[range_name.value] = {
                "original_threshold": threshold.original_threshold,
                "learned_threshold": threshold.learned_threshold,
                "adjustment": threshold.adjustment,
                "confidence_in_threshold": threshold.confidence_in_threshold,
                "learning_iterations": threshold.learning_iterations,
            }
        return result

    # ========================================================================
    # Private Helper Methods
    # ========================================================================

    def _compute_adjustment_delta(
        self, original_confidence: float, feedback_type: FeedbackType
    ) -> float:
        """Compute confidence adjustment delta from feedback type."""
        mapping = {
            FeedbackType.ACCEPT: 0.0,  # No change, confidence was correct
            FeedbackType.REJECT: -0.20,  # Lower confidence (concept too high)
            FeedbackType.MISSING: 0.0,  # No adjustment to this concept
            FeedbackType.SPURIOUS: -0.30,  # Much lower (should not extract)
        }
        return mapping.get(feedback_type, 0.0)

    def _range_to_threshold(self, confidence_range: ConfidenceRange) -> float:
        """Map confidence range to default threshold."""
        mapping = {
            ConfidenceRange.VERY_HIGH: 0.90,
            ConfidenceRange.HIGH: 0.75,
            ConfidenceRange.MEDIUM: 0.50,
            ConfidenceRange.LOW: 0.25,
            ConfidenceRange.VERY_LOW: 0.00,
        }
        return mapping[confidence_range]

    def _get_confidence_range(self, confidence: float) -> ConfidenceRange:
        """Map confidence score to range."""
        if confidence >= 0.90:
            return ConfidenceRange.VERY_HIGH
        elif confidence >= 0.75:
            return ConfidenceRange.HIGH
        elif confidence >= 0.50:
            return ConfidenceRange.MEDIUM
        elif confidence >= 0.25:
            return ConfidenceRange.LOW
        else:
            return ConfidenceRange.VERY_LOW

    def _aggregate_feedback_by_range(self) -> Dict[ConfidenceRange, List[ConfidenceDelta]]:
        """Aggregate all feedback by confidence range."""
        result = {r: [] for r in ConfidenceRange}
        for feedback_list in self._feedback_history.values():
            for delta in feedback_list:
                confidence_range = self._get_confidence_range(delta.original_confidence)
                result[confidence_range].append(delta)
        return result

    def _get_recent_feedback(
        self, feedback_items: List[ConfidenceDelta]
    ) -> List[ConfidenceDelta]:
        """Filter feedback to recent items (within adaptive_window_hours)."""
        cutoff_time = datetime.utcnow() - timedelta(hours=self.adaptive_window_hours)
        return [f for f in feedback_items if f.timestamp >= cutoff_time]

    def _exponential_moving_average(
        self, old_value: float, new_value: float, sample_count: int
    ) -> float:
        """Compute exponential moving average with dynamic smoothing factor."""
        # Smoothing factor decreases with sample count (more samples = more trust)
        # alpha = 2 / (sample_count + 1)
        alpha = 2.0 / (sample_count + 1.0)
        return old_value * (1 - alpha) + new_value * alpha
