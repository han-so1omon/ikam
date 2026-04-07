"""
FastAPI router for adaptive threshold learning endpoints.

Provides HTTP access to the AdaptiveThresholdLearner, allowing clients to:
- Submit feedback on concept extraction results
- Update learned thresholds
- Query metrics and learned thresholds
- Clear feedback history
"""

from datetime import datetime
from typing import Optional, Dict, List
from fastapi import APIRouter, Depends, HTTPException, status
from pydantic import BaseModel, Field, validator

from modelado.sequencer.adaptive_thresholds import (
    AdaptiveThresholdLearner,
    FeedbackType,
    ConfidenceRange,
    ConfidenceDelta,
    LearnedThreshold,
    AdaptiveThresholdMetrics,
)


# ============================================================================
# Request Models
# ============================================================================


class FeedbackRequest(BaseModel):
    """Submit feedback on a concept's confidence."""

    concept_id: str = Field(..., min_length=1, max_length=256)
    original_confidence: float = Field(..., ge=0.0, le=1.0)
    feedback_type: str = Field(..., description="accept | reject | missing | spurious")
    reason: str = Field("", max_length=500, description="Optional explanation")

    @validator("feedback_type")
    def validate_feedback_type(cls, v):
        """Validate feedback type."""
        valid_types = {ft.value for ft in FeedbackType}
        if v not in valid_types:
            raise ValueError(f"feedback_type must be one of {valid_types}")
        return v


class BatchFeedbackRequest(BaseModel):
    """Submit feedback for multiple concepts."""

    feedback_items: List[FeedbackRequest] = Field(..., min_items=1, max_items=100)


class UpdateThresholdsRequest(BaseModel):
    """Request to update learned thresholds."""

    learning_rate: Optional[float] = Field(
        None, ge=0.0, le=1.0, description="Override default learning rate"
    )


class ClearFeedbackRequest(BaseModel):
    """Request to clear feedback history."""

    concept_id: Optional[str] = Field(
        None, description="If provided, clear only this concept's feedback"
    )


# ============================================================================
# Response Models
# ============================================================================


class ConfidenceDeltaResponse(BaseModel):
    """Recorded feedback adjustment."""

    concept_id: str
    original_confidence: float
    user_feedback: str
    adjustment_delta: float
    reason: str
    timestamp: datetime

    @classmethod
    def from_domain(cls, delta: ConfidenceDelta) -> "ConfidenceDeltaResponse":
        """Convert domain model to response."""
        return cls(
            concept_id=delta.concept_id,
            original_confidence=delta.original_confidence,
            user_feedback=delta.user_feedback.value,
            adjustment_delta=delta.adjustment_delta,
            reason=delta.reason,
            timestamp=delta.timestamp,
        )


class ThresholdScoreResponse(BaseModel):
    """Aggregated feedback score for a confidence range."""

    range_name: str
    mean_satisfaction: float
    feedback_count: int
    recent_feedback_count: int
    last_updated: datetime


class LearnedThresholdResponse(BaseModel):
    """Learned optimal confidence threshold."""

    range_name: str
    original_threshold: float
    learned_threshold: float
    adjustment: float
    confidence_in_threshold: float
    learning_iterations: int
    last_updated: datetime

    @classmethod
    def from_domain(cls, threshold: LearnedThreshold) -> "LearnedThresholdResponse":
        """Convert domain model to response."""
        return cls(
            range_name=threshold.range_name.value,
            original_threshold=threshold.original_threshold,
            learned_threshold=threshold.learned_threshold,
            adjustment=threshold.adjustment,
            confidence_in_threshold=threshold.confidence_in_threshold,
            learning_iterations=threshold.learning_iterations,
            last_updated=threshold.last_updated,
        )


class FeedbackResponse(BaseModel):
    """Response after recording feedback."""

    feedback: ConfidenceDeltaResponse
    message: str = "Feedback recorded successfully"


class BatchFeedbackResponse(BaseModel):
    """Response after recording batch feedback."""

    total_items: int
    successful: int
    failed: int
    feedback_items: List[ConfidenceDeltaResponse] = []


class UpdateThresholdsResponse(BaseModel):
    """Response after updating thresholds."""

    learned_thresholds: Dict[str, dict]
    converged: bool = False
    message: str = "Thresholds updated successfully"


class AdaptiveThresholdMetricsResponse(BaseModel):
    """Aggregate metrics for adaptive threshold learning."""

    total_feedback: int
    total_concepts_evaluated: int
    mean_feedback_score: float
    feedback_distribution: Dict[str, int]
    confidence_distribution: Dict[str, int]
    learning_converged: bool
    convergence_delta: float
    last_updated: datetime

    @classmethod
    def from_domain(cls, metrics: AdaptiveThresholdMetrics) -> "AdaptiveThresholdMetricsResponse":
        """Convert domain model to response."""
        return cls(
            total_feedback=metrics.total_feedback,
            total_concepts_evaluated=metrics.total_concepts_evaluated,
            mean_feedback_score=metrics.mean_feedback_score,
            feedback_distribution=metrics.feedback_distribution,
            confidence_distribution=metrics.confidence_distribution,
            learning_converged=metrics.learning_converged,
            convergence_delta=metrics.convergence_delta,
            last_updated=metrics.last_updated,
        )


class EffectiveThresholdResponse(BaseModel):
    """Effective threshold for a confidence range."""

    confidence_range: str
    effective_threshold: float
    is_learned: bool = False
    confidence_in_threshold: float = 0.0


class ClearFeedbackResponse(BaseModel):
    """Response after clearing feedback."""

    cleared_count: int
    message: str = "Feedback cleared successfully"


# ============================================================================
# FastAPI Router Factory
# ============================================================================


def create_adaptive_thresholds_router(
    get_learner: callable,
) -> APIRouter:
    """
    Create FastAPI router for adaptive threshold learning.

    Args:
        get_learner: Dependency function returning AdaptiveThresholdLearner instance

    Returns:
        APIRouter with adaptive threshold endpoints
    """
    router = APIRouter(prefix="/api/model/thresholds", tags=["adaptive-thresholds"])

    # ========================================================================
    # Endpoints
    # ========================================================================

    @router.post("/feedback", response_model=FeedbackResponse, status_code=status.HTTP_201_CREATED)
    async def record_feedback(
        request: FeedbackRequest,
        learner: AdaptiveThresholdLearner = Depends(get_learner),
    ) -> FeedbackResponse:
        """
        Record user feedback on a concept's confidence.

        **Request:**
        ```json
        {
          "concept_id": "revenue_forecasting",
          "original_confidence": 0.92,
          "feedback_type": "accept",
          "reason": "Confidence was accurate"
        }
        ```

        **Response:**
        ```json
        {
          "feedback": {
            "concept_id": "revenue_forecasting",
            "original_confidence": 0.92,
            "user_feedback": "accept",
            "adjustment_delta": 0.0,
            "timestamp": "2025-12-10T10:30:00Z"
          },
          "message": "Feedback recorded successfully"
        }
        ```

        **Feedback Types:**
        - `accept`: Concept confidence was appropriate
        - `reject`: Confidence too high, should be lower
        - `missing`: Concept should have been extracted
        - `spurious`: Concept should not have been extracted

        **Returns:**
        - 201: Feedback recorded successfully
        - 400: Invalid request (bad concept_id, confidence, feedback_type)
        """
        try:
            delta = await learner.record_feedback(
                concept_id=request.concept_id,
                original_confidence=request.original_confidence,
                feedback_type=FeedbackType(request.feedback_type),
                reason=request.reason,
            )
            return FeedbackResponse(feedback=ConfidenceDeltaResponse.from_domain(delta))
        except ValueError as e:
            raise HTTPException(status_code=400, detail=str(e))

    @router.post(
        "/feedback/batch",
        response_model=BatchFeedbackResponse,
        status_code=status.HTTP_201_CREATED,
    )
    async def record_batch_feedback(
        request: BatchFeedbackRequest,
        learner: AdaptiveThresholdLearner = Depends(get_learner),
    ) -> BatchFeedbackResponse:
        """
        Record feedback for multiple concepts in one request.

        **Request:**
        ```json
        {
          "feedback_items": [
            {
              "concept_id": "revenue_forecasting",
              "original_confidence": 0.92,
              "feedback_type": "accept"
            },
            {
              "concept_id": "unit_economics",
              "original_confidence": 0.65,
              "feedback_type": "reject",
              "reason": "Confidence too high"
            }
          ]
        }
        ```

        **Response:**
        ```json
        {
          "total_items": 2,
          "successful": 2,
          "failed": 0,
          "feedback_items": [...]
        }
        ```

        **Returns:**
        - 201: Batch processed (some or all successful)
        - 400: Invalid batch request
        """
        feedback_items = []
        failed_count = 0

        for item in request.feedback_items:
            try:
                delta = await learner.record_feedback(
                    concept_id=item.concept_id,
                    original_confidence=item.original_confidence,
                    feedback_type=FeedbackType(item.feedback_type),
                    reason=item.reason,
                )
                feedback_items.append(ConfidenceDeltaResponse.from_domain(delta))
            except ValueError:
                failed_count += 1

        return BatchFeedbackResponse(
            total_items=len(request.feedback_items),
            successful=len(feedback_items),
            failed=failed_count,
            feedback_items=feedback_items,
        )

    @router.post(
        "/update",
        response_model=UpdateThresholdsResponse,
        status_code=status.HTTP_200_OK,
    )
    async def update_thresholds(
        request: UpdateThresholdsRequest,
        learner: AdaptiveThresholdLearner = Depends(get_learner),
    ) -> UpdateThresholdsResponse:
        """
        Update learned thresholds based on accumulated feedback.

        Algorithm:
        1. Aggregate feedback per confidence range
        2. Compute satisfaction scores (% accept feedback)
        3. Adjust thresholds by satisfaction × learning_rate
        4. Update confidence_in_threshold based on feedback count
        5. Detect convergence (threshold changes < 0.01)

        **Response:**
        ```json
        {
          "learned_thresholds": {
            "very_high": {
              "original_threshold": 0.90,
              "learned_threshold": 0.92,
              "adjustment": 0.02,
              "confidence_in_threshold": 0.85,
              "learning_iterations": 3
            }
          },
          "converged": false,
          "message": "Thresholds updated successfully"
        }
        ```

        **Returns:**
        - 200: Thresholds updated
        - 400: Invalid parameters
        """
        try:
            thresholds = await learner.update_thresholds()
            metrics = await learner.get_metrics()
            return UpdateThresholdsResponse(
                learned_thresholds=learner.get_learned_thresholds_dict(),
                converged=metrics.learning_converged,
            )
        except Exception as e:
            raise HTTPException(status_code=500, detail=str(e))

    @router.get(
        "/metrics",
        response_model=AdaptiveThresholdMetricsResponse,
        status_code=status.HTTP_200_OK,
    )
    async def get_metrics(
        learner: AdaptiveThresholdLearner = Depends(get_learner),
    ) -> AdaptiveThresholdMetricsResponse:
        """
        Get aggregate metrics on adaptive threshold learning.

        **Response:**
        ```json
        {
          "total_feedback": 42,
          "total_concepts_evaluated": 18,
          "mean_feedback_score": 0.81,
          "feedback_distribution": {
            "accept": 34,
            "reject": 6,
            "missing": 1,
            "spurious": 1
          },
          "confidence_distribution": {
            "very_high": 15,
            "high": 18,
            "medium": 8,
            "low": 1
          },
          "learning_converged": false,
          "convergence_delta": 0.02,
          "last_updated": "2025-12-10T10:35:00Z"
        }
        ```

        **Returns:**
        - 200: Metrics retrieved successfully
        """
        metrics = await learner.get_metrics()
        return AdaptiveThresholdMetricsResponse.from_domain(metrics)

    @router.get(
        "/thresholds",
        response_model=Dict[str, dict],
        status_code=status.HTTP_200_OK,
    )
    async def get_learned_thresholds(
        learner: AdaptiveThresholdLearner = Depends(get_learner),
    ) -> Dict[str, dict]:
        """
        Get all learned thresholds.

        **Response:**
        ```json
        {
          "very_high": {
            "original_threshold": 0.90,
            "learned_threshold": 0.92,
            "adjustment": 0.02,
            "confidence_in_threshold": 0.85,
            "learning_iterations": 3
          },
          "high": {
            "original_threshold": 0.75,
            "learned_threshold": 0.76,
            ...
          }
        }
        ```

        **Returns:**
        - 200: Thresholds retrieved
        """
        return learner.get_learned_thresholds_dict()

    @router.get(
        "/thresholds/{confidence_range}",
        response_model=EffectiveThresholdResponse,
        status_code=status.HTTP_200_OK,
    )
    async def get_effective_threshold(
        confidence_range: str,
        min_confidence: float = 0.50,
        learner: AdaptiveThresholdLearner = Depends(get_learner),
    ) -> EffectiveThresholdResponse:
        """
        Get effective threshold for a confidence range.

        Uses learned threshold only if confidence_in_threshold >= min_confidence.
        Otherwise falls back to original threshold.

        **Query Parameters:**
        - `confidence_range`: very_high, high, medium, low, very_low
        - `min_confidence`: Minimum confidence to use learned threshold (0.0-1.0)

        **Response:**
        ```json
        {
          "confidence_range": "high",
          "effective_threshold": 0.76,
          "is_learned": true,
          "confidence_in_threshold": 0.85
        }
        ```

        **Returns:**
        - 200: Threshold retrieved
        - 404: Invalid confidence_range
        """
        try:
            conf_range = ConfidenceRange(confidence_range)
        except ValueError:
            raise HTTPException(
                status_code=404,
                detail=f"Unknown confidence_range: {confidence_range}",
            )

        effective = await learner.get_effective_threshold(conf_range, min_confidence)
        learned_threshold = learner._learned_thresholds[conf_range]

        return EffectiveThresholdResponse(
            confidence_range=confidence_range,
            effective_threshold=effective,
            is_learned=effective == learned_threshold.learned_threshold,
            confidence_in_threshold=learned_threshold.confidence_in_threshold,
        )

    @router.delete(
        "/feedback",
        response_model=ClearFeedbackResponse,
        status_code=status.HTTP_200_OK,
    )
    async def clear_feedback(
        request: ClearFeedbackRequest,
        learner: AdaptiveThresholdLearner = Depends(get_learner),
    ) -> ClearFeedbackResponse:
        """
        Clear feedback history.

        **Request:**
        ```json
        {
          "concept_id": "revenue_forecasting"
        }
        ```

        If concept_id is omitted, clears all feedback.

        **Response:**
        ```json
        {
          "cleared_count": 5,
          "message": "Feedback cleared successfully"
        }
        ```

        **Returns:**
        - 200: Feedback cleared
        """
        cleared = await learner.clear_feedback(request.concept_id)
        return ClearFeedbackResponse(cleared_count=cleared)

    return router
