"""Tests for scenario intent parsing with semantic evaluation."""

import os
import pytest
from unittest.mock import AsyncMock, MagicMock, patch
from modelado.sequencer.simulator import (
    parse_scenario_intent_semantic,
    parse_scenario_intent_fallback,
    parse_scenario_intent,
    _extract_scenario_parameters,
    _map_evaluator_to_operation,
    _extract_phases_from_description,
)


# ============================================================================
# Tests: Fallback Keyword-Based Intent Parsing
# ============================================================================


class TestScenarioIntentFallback:
    """Test fallback keyword-based scenario intent parsing."""

    def test_fallback_hire_contractor_basic(self):
        """Test basic contractor hiring scenario detection."""
        result = parse_scenario_intent_fallback("Hire a contractor for phase 2")

        assert result["operation"] == "resource_substitution"
        assert result["confidence"] == 0.75  # Lower fallback confidence
        assert "phase-2" in result["target_phases"]
        assert result["parameters"]["role"] == "contractor"
        assert result["parameters"]["cost_multiplier"] == 1.4

    def test_fallback_hire_contractor_offshore(self):
        """Test contractor with offshore location."""
        result = parse_scenario_intent_fallback(
            "Hire a contractor from India for phase 1"
        )

        assert result["operation"] == "resource_substitution"
        assert result["parameters"]["cost_multiplier"] == 1.2  # Cheaper offshore

    def test_fallback_hire_senior_contractor(self):
        """Test hiring senior/expert contractor."""
        result = parse_scenario_intent_fallback(
            "Get an expert consultant for phase 3"
        )

        assert result["operation"] == "resource_substitution"
        assert result["parameters"]["cost_multiplier"] == 1.6  # More expensive expert

    def test_fallback_multiple_contractors(self):
        """Test hiring multiple contractors."""
        result = parse_scenario_intent_fallback(
            "Hire 2 contractors for implementation"
        )

        assert result["operation"] == "resource_substitution"
        assert result["parameters"]["effort_reduction"] == 0.75  # More efficient

    def test_fallback_reduce_scope(self):
        """Test scope reduction scenario."""
        result = parse_scenario_intent_fallback("Reduce scope by 25%")

        assert result["operation"] == "scope_reduction"
        assert result["confidence"] == 0.80
        assert result["parameters"]["reduction_percent"] == 0.25
        assert result["target_phases"] == ["all"]

    def test_fallback_cost_optimization(self):
        """Test cost optimization scenario."""
        result = parse_scenario_intent_fallback("Save 10k on the project")

        assert result["operation"] == "cost_optimization"
        assert result["confidence"] == 0.70
        assert result["parameters"]["target_savings"] == 10000

    def test_fallback_parallelize(self):
        """Test parallelize phases scenario."""
        result = parse_scenario_intent_fallback("Parallelize phases 1-3")

        assert result["operation"] == "parallelize"
        assert result["confidence"] == 0.78
        assert "phase-1" in result["target_phases"]
        assert "phase-3" in result["target_phases"]

    def test_fallback_quality_enhancement(self):
        """Test quality/testing scenario."""
        result = parse_scenario_intent_fallback(
            "Add comprehensive testing to phase 2"
        )

        assert result["operation"] == "quality_enhancement"
        assert result["confidence"] == 0.72
        assert result["parameters"]["testing_depth"] == "comprehensive"

    def test_fallback_unknown_scenario(self):
        """Test unknown/unrecognized scenario."""
        result = parse_scenario_intent_fallback(
            "Do something really weird with the timeline"
        )

        assert result["operation"] == "custom_adjustment"
        assert result["confidence"] == 0.50  # Low confidence for unknown
        assert result["features"] == []

    def test_fallback_all_results_have_required_fields(self):
        """Test that all fallback results have required fields."""
        scenarios = [
            "Hire contractor",
            "Cut scope",
            "Parallelize phases",
            "Test everything",
            "Something unknown",
        ]

        for scenario in scenarios:
            result = parse_scenario_intent_fallback(scenario)

            assert "operation" in result
            assert "confidence" in result
            assert 0.0 <= result["confidence"] <= 1.0
            assert "target_phases" in result
            assert "parameters" in result
            assert "features" in result
            assert "reasoning" in result


# ============================================================================
# Tests: Backward Compatibility (Sync API)
# ============================================================================


class TestScenarioIntentBackwardCompat:
    """Test backward compatibility wrapper."""

    def test_sync_parse_scenario_intent_basic(self):
        """Test sync wrapper uses fallback implementation."""
        result = parse_scenario_intent("Hire contractor for phase 2")

        # Should use fallback (no async capability in sync context)
        assert result["operation"] == "resource_substitution"
        assert result["confidence"] <= 0.75  # Fallback confidence

    def test_sync_parse_scenario_intent_with_phases(self):
        """Test sync wrapper extracts phases correctly."""
        result = parse_scenario_intent("Parallelize phases 1-3")

        assert result["operation"] == "parallelize"
        assert len(result["target_phases"]) > 0


# ============================================================================
# Tests: Semantic API (Async with Mocking)
# ============================================================================


class TestScenarioIntentSemantic:
    """Test semantic-based intent parsing with mocked SemanticEngine."""

    @pytest.mark.asyncio
    async def test_semantic_hire_contractor_via_engine(self):
        """Test semantic parsing with mocked SemanticEngine."""
        # Create mock SemanticEngine
        mock_engine = AsyncMock()
        mock_evaluation = MagicMock()
        mock_evaluation.evaluator_name = "EconomicFunctionEvaluator"
        mock_evaluation.evaluator_confidence = 0.92
        mock_evaluation.semantic_features = {
            "resource_related": True,
            "cost_related": False,
            "scope_related": False,
        }
        mock_evaluation.reasoning = "Detected resource substitution intent"

        mock_engine.evaluate = AsyncMock(return_value=mock_evaluation)

        result = await parse_scenario_intent_semantic(
            "Hire a contractor for phase 2",
            semantic_engine=mock_engine,
        )

        assert result["operation"] == "resource_substitution"
        assert result["confidence"] == 0.92
        assert "phase-2" in result["target_phases"]
        assert result["evaluator"] == "EconomicFunctionEvaluator"

    @pytest.mark.asyncio
    async def test_semantic_scope_reduction_via_engine(self):
        """Test semantic detection of scope reduction."""
        mock_engine = AsyncMock()
        mock_evaluation = MagicMock()
        mock_evaluation.evaluator_name = "EconomicFunctionEvaluator"
        mock_evaluation.evaluator_confidence = 0.85
        mock_evaluation.semantic_features = {
            "resource_related": False,
            "cost_related": False,
            "scope_related": True,
        }
        mock_evaluation.reasoning = "Detected scope reduction intent"

        mock_engine.evaluate = AsyncMock(return_value=mock_evaluation)

        result = await parse_scenario_intent_semantic(
            "Reduce scope by 30%",
            semantic_engine=mock_engine,
        )

        assert result["operation"] == "scope_reduction"
        assert result["confidence"] == 0.85

    @pytest.mark.asyncio
    async def test_semantic_fallback_on_engine_unavailable(self):
        """Test fallback when SemanticEngine not available."""
        with patch.dict(os.environ, {"OPENAI_API_KEY": ""}):
            result = await parse_scenario_intent_semantic(
                "Hire contractor",
                semantic_engine=None,
            )

        # Should fall back to keyword parsing
        assert result["operation"] == "resource_substitution"
        assert result["confidence"] <= 0.75

    @pytest.mark.asyncio
    async def test_semantic_fallback_on_engine_error(self):
        """Test fallback when SemanticEngine raises error."""
        mock_engine = AsyncMock()
        mock_engine.evaluate = AsyncMock(side_effect=RuntimeError("API error"))

        result = await parse_scenario_intent_semantic(
            "Hire contractor",
            semantic_engine=mock_engine,
        )

        # Should fall back to keyword parsing on error
        assert result["operation"] == "resource_substitution"
        assert result["confidence"] <= 0.75


# ============================================================================
# Tests: Helper Functions
# ============================================================================


class TestExtractPhases:
    """Test phase extraction from descriptions."""

    def test_extract_single_phase(self):
        """Test extracting single phase reference."""
        phases = _extract_phases_from_description("for phase 2")
        assert "phase-2" in phases

    def test_extract_phase_range(self):
        """Test extracting phase range."""
        phases = _extract_phases_from_description("parallelize phases 1-3")
        assert "phase-1" in phases
        assert "phase-2" in phases
        assert "phase-3" in phases

    def test_extract_multiple_phase_references(self):
        """Test extracting multiple phase references."""
        phases = _extract_phases_from_description(
            "Hire contractor for phase 1 and phase 3"
        )
        assert "phase-1" in phases
        assert "phase-3" in phases
        assert "phase-2" not in phases

    def test_extract_no_phases(self):
        """Test with no phase references."""
        phases = _extract_phases_from_description("Add testing")
        assert len(phases) == 0

    def test_extract_deduplicates_phases(self):
        """Test that duplicate phase references are deduplicated."""
        phases = _extract_phases_from_description("phase 1 and phases 1-2")
        assert phases.count("phase-1") == 1
        assert "phase-2" in phases


class TestMapEvaluatorToOperation:
    """Test mapping SemanticEngine evaluator to operation type."""

    def test_map_economic_to_resource_substitution(self):
        """Test mapping economic evaluator with resource features."""
        operation = _map_evaluator_to_operation(
            "EconomicFunctionEvaluator",
            {"resource_related": True, "cost_related": False},
            "Hire contractor",
        )
        assert operation == "resource_substitution"

    def test_map_economic_to_cost_optimization(self):
        """Test mapping economic evaluator with cost features."""
        operation = _map_evaluator_to_operation(
            "EconomicFunctionEvaluator",
            {"resource_related": False, "cost_related": True},
            "Save 10k",
        )
        assert operation == "cost_optimization"

    def test_map_economic_to_scope_reduction(self):
        """Test mapping economic evaluator with scope features."""
        operation = _map_evaluator_to_operation(
            "EconomicFunctionEvaluator",
            {"resource_related": False, "scope_related": True},
            "Reduce scope",
        )
        assert operation == "scope_reduction"

    def test_map_story_evaluator(self):
        """Test mapping story evaluator."""
        operation = _map_evaluator_to_operation(
            "StoryOperationEvaluator",
            {"narrative": True},
            "Create slides",
        )
        assert operation == "story_update"

    def test_map_constraint_to_parallelize(self):
        """Test mapping constraint evaluator with parallel scenario."""
        operation = _map_evaluator_to_operation(
            "ConstraintOperationEvaluator",
            {"parallelization": True},
            "Parallelize phases 1-2",
        )
        assert operation == "parallelize"

    def test_map_unknown_evaluator(self):
        """Test mapping unknown evaluator defaults to custom."""
        operation = _map_evaluator_to_operation(
            "UnknownEvaluator",
            {},
            "Do something",
        )
        assert operation == "custom_adjustment"

    def test_map_none_evaluator(self):
        """Test mapping None evaluator."""
        operation = _map_evaluator_to_operation(None, {}, "Something")
        assert operation == "custom_adjustment"


class TestExtractScenarioParameters:
    """Test parameter extraction from scenario descriptions."""

    def test_extract_contractor_parameters_basic(self):
        """Test extracting contractor parameters."""
        params = _extract_scenario_parameters(
            "Hire contractor",
            {"resource_related": True},
            "resource_substitution",
        )

        assert params["role"] == "contractor"
        assert params["cost_multiplier"] == 1.4
        assert params["effort_reduction"] == 0.85
        assert params["risk_adjustment"] == 0.05

    def test_extract_contractor_offshore(self):
        """Test offshore contractor parameters."""
        params = _extract_scenario_parameters(
            "Hire contractor from India",
            {"resource_related": True},
            "resource_substitution",
        )

        assert params["cost_multiplier"] == 1.2

    def test_extract_contractor_senior(self):
        """Test senior contractor parameters."""
        params = _extract_scenario_parameters(
            "Hire expert senior developer",
            {"resource_related": True},
            "resource_substitution",
        )

        assert params["cost_multiplier"] == 1.6

    def test_extract_scope_parameters(self):
        """Test extracting scope reduction parameters."""
        params = _extract_scenario_parameters(
            "Reduce scope by 25%",
            {"scope_related": True},
            "scope_reduction",
        )

        assert params["reduction_percent"] == 0.25

    def test_extract_cost_parameters(self):
        """Test extracting cost optimization parameters."""
        params = _extract_scenario_parameters(
            "Save 20k",
            {"cost_related": True},
            "cost_optimization",
        )

        assert params["target_savings"] == 20000
        assert params["preserve_timeline"] is True

    def test_extract_quality_parameters(self):
        """Test extracting quality enhancement parameters."""
        params = _extract_scenario_parameters(
            "Add comprehensive testing",
            {"quality_related": True},
            "quality_enhancement",
        )

        assert params["testing_depth"] == "comprehensive"
        assert params["effort_multiplier"] == 1.3
        assert params["cost_multiplier"] == 1.2

    def test_extract_parallelize_parameters(self):
        """Test extracting parallelize parameters."""
        params = _extract_scenario_parameters(
            "Parallelize phases",
            {},
            "parallelize",
        )

        assert params["critical_path_focus"] is True
        assert params["resource_constraint"] == "none"


# ============================================================================
# Integration Tests: Full Flow
# ============================================================================


class TestIntentParsingIntegration:
    """Integration tests for full intent parsing flow."""

    def test_full_flow_fallback_hire_contractor(self):
        """Test full flow: parse → validate → result."""
        result = parse_scenario_intent("Hire contractor for phase 2")

        # Result should be valid dict with all required fields
        assert isinstance(result, dict)
        assert all(
            k in result
            for k in [
                "operation",
                "confidence",
                "target_phases",
                "parameters",
            ]
        )

        # Contractor-specific validations
        assert result["operation"] == "resource_substitution"
        assert 0.5 <= result["confidence"] <= 1.0
        assert "phase-2" in result["target_phases"]
        assert "role" in result["parameters"]

    def test_full_flow_fallback_scope_reduction(self):
        """Test full flow for scope reduction."""
        result = parse_scenario_intent("Cut scope by 30%")

        assert result["operation"] == "scope_reduction"
        assert result["parameters"]["reduction_percent"] == 0.30

    @pytest.mark.asyncio
    async def test_full_flow_semantic_with_mock(self):
        """Test full semantic flow with mocked engine."""
        mock_engine = AsyncMock()
        mock_eval = MagicMock()
        mock_eval.evaluator_name = "EconomicFunctionEvaluator"
        mock_eval.evaluator_confidence = 0.88
        mock_eval.semantic_features = {
            "resource_related": True,
            "cost_related": False,
            "scope_related": False,
        }
        mock_eval.reasoning = "Resource substitution detected"
        mock_engine.evaluate = AsyncMock(return_value=mock_eval)

        result = await parse_scenario_intent_semantic(
            "Hire 2 contractors for phase 1",
            semantic_engine=mock_engine,
        )

        # Verify full result structure
        assert result["operation"] == "resource_substitution"
        assert result["confidence"] == 0.88
        assert result["evaluator"] == "EconomicFunctionEvaluator"
        assert "phase-1" in result["target_phases"]
        assert result["parameters"]["effort_reduction"] == 0.75  # 2 contractors
        assert "features" in result
        assert "reasoning" in result
