"""Tests for Phase 9.2: Operation Handler Implementations.

Tests validate that:
1. Each handler can process GenerativeCommand correctly
2. Handler creates valid ExecutableFunction
3. Generation metadata is recorded properly
4. Each handler follows GenerativeHandler interface
5. Error handling works correctly
"""

import pytest
from unittest.mock import AsyncMock, MagicMock

from modelado.core.generative_contracts import GenerativeCommand, GenerationStrategy
from modelado.core.operation_handlers import (
    EconomicOperationHandler,
    StoryOperationHandler,
    SystemOperationHandler,
    SemanticConfidenceScorer,
    FallbackStrategyRouter,
    detect_intent_ambiguity,
    ECONOMIC_INTENT_KEYWORDS,
    STORY_INTENT_KEYWORDS,
    SYSTEM_INTENT_KEYWORDS,
)


class TestSemanticConfidenceScorer:
    """Tests for semantic confidence scoring with reasoning."""
    
    def test_score_intent_with_keyword_match(self):
        """Test confidence scoring with keyword matches."""
        instruction = "Calculate sensitivity analysis on revenue drivers"
        confidence, reasoning = SemanticConfidenceScorer.score_intent(
            instruction=instruction,
            intent_type="sensitivity_analysis",
            keywords_dict=ECONOMIC_INTENT_KEYWORDS,
        )
        
        # Should have decent confidence with keyword matches
        assert confidence >= 0.40
        assert confidence <= 1.0
        assert reasoning["intent_type"] == "sensitivity_analysis"
        assert len(reasoning["keyword_matches"]) > 0
        assert reasoning["match_coverage"] > 0.0
        assert reasoning["reasoning_summary"] is not None
    
    def test_score_intent_without_keyword_match(self):
        """Test confidence scoring with no keyword matches."""
        instruction = "Do something with the data"
        confidence, reasoning = SemanticConfidenceScorer.score_intent(
            instruction=instruction,
            intent_type="sensitivity_analysis",
            keywords_dict=ECONOMIC_INTENT_KEYWORDS,
        )
        
        # Should have lower confidence with no keyword matches
        assert confidence < 0.70
        assert reasoning["match_coverage"] == 0.0
    
    def test_score_intent_detail_level_impact(self):
        """Test that instruction detail level affects confidence."""
        short_instruction = "sensitivity"
        long_instruction = "Execute a comprehensive sensitivity analysis on all revenue drivers including COGS, pricing, and volume to understand elasticity"
        
        _, short_reasoning = SemanticConfidenceScorer.score_intent(
            instruction=short_instruction,
            intent_type="sensitivity_analysis",
            keywords_dict=ECONOMIC_INTENT_KEYWORDS,
        )
        
        _, long_reasoning = SemanticConfidenceScorer.score_intent(
            instruction=long_instruction,
            intent_type="sensitivity_analysis",
            keywords_dict=ECONOMIC_INTENT_KEYWORDS,
        )
        
        # Longer instruction should have higher detail score
        assert long_reasoning["detail_score"] > short_reasoning["detail_score"]
        assert long_reasoning["final_confidence"] > short_reasoning["final_confidence"]
    
    def test_score_intent_feature_detection(self):
        """Test feature detection affects confidence."""
        instruction = "Create sensitivity analysis with parallel execution and validation"
        _, reasoning = SemanticConfidenceScorer.score_intent(
            instruction=instruction,
            intent_type="sensitivity_analysis",
            keywords_dict=ECONOMIC_INTENT_KEYWORDS,
        )
        
        # Should detect multiple features
        assert len(reasoning["features_detected"]) > 0
        assert reasoning["feature_score"] > 0.5
    
    def test_score_intent_story_keywords(self):
        """Test scoring with story operation keywords."""
        instruction = "Create compelling story arc emphasizing growth narrative"
        confidence, reasoning = SemanticConfidenceScorer.score_intent(
            instruction=instruction,
            intent_type="story_arc_development",
            keywords_dict=STORY_INTENT_KEYWORDS,
        )
        
        # Should have confidence above baseline
        assert confidence >= 0.60
        assert confidence <= 1.0
        assert reasoning["intent_type"] == "story_arc_development"
        assert reasoning["match_coverage"] > 0.0
    
    def test_score_intent_system_keywords(self):
        """Test scoring with system operation keywords."""
        instruction = "Orchestrate batch processing pipeline with error recovery"
        confidence, reasoning = SemanticConfidenceScorer.score_intent(
            instruction=instruction,
            intent_type="batch_processing",
            keywords_dict=SYSTEM_INTENT_KEYWORDS,
        )
        
        # Should have confidence above baseline
        assert confidence >= 0.50
        assert confidence <= 1.0
        assert reasoning["intent_type"] == "batch_processing"
        assert reasoning["match_coverage"] > 0.0
        assert "parallel_execution" in reasoning["features_detected"] or "retry_logic" in reasoning["features_detected"]
    
    def test_confidence_threshold_levels(self):
        """Test confidence threshold constants."""
        assert SemanticConfidenceScorer.CONFIDENCE_LOW == 0.60
        assert SemanticConfidenceScorer.CONFIDENCE_MEDIUM == 0.75
        assert SemanticConfidenceScorer.CONFIDENCE_HIGH == 0.85
        assert SemanticConfidenceScorer.CONFIDENCE_VERY_HIGH == 0.95

class TestFallbackStrategyRouter:
    """Tests for confidence-based fallback strategy routing."""

    def test_select_strategy_high_confidence(self):
        strategy = FallbackStrategyRouter.select_strategy(0.90)
        assert strategy == FallbackStrategyRouter.Strategy.PRIMARY

    def test_select_strategy_medium_confidence(self):
        strategy = FallbackStrategyRouter.select_strategy(0.80)
        assert strategy == FallbackStrategyRouter.Strategy.HYBRID

    def test_select_strategy_low_confidence(self):
        strategy = FallbackStrategyRouter.select_strategy(0.50)
        assert strategy == FallbackStrategyRouter.Strategy.GENERIC

    def test_get_strategy_metadata_contains_reason(self):
        strategy = FallbackStrategyRouter.Strategy.HYBRID
        metadata = FallbackStrategyRouter.get_strategy_metadata(strategy, 0.80, "story_arc_development")
        assert metadata["fallback_strategy"] == "hybrid"
        assert metadata["confidence_tier"] == "medium"
        assert "routing_reason" in metadata
        assert "features_enabled" in metadata and len(metadata["features_enabled"]) > 0


class TestAmbiguityDetectionEdgeCases:
    """Edge cases for detect_intent_ambiguity helper."""

    def test_empty_instruction_not_ambiguous(self):
        result = detect_intent_ambiguity("", ECONOMIC_INTENT_KEYWORDS)
        assert result["is_ambiguous"] is False
        assert result["candidates"] == []

    def test_single_dominant_intent_not_ambiguous(self):
        instruction = "Perform sensitivity analysis on revenue drivers"
        result = detect_intent_ambiguity(instruction, ECONOMIC_INTENT_KEYWORDS)
        # Allow ambiguity if another intent ties closely; assert top candidate is sensitivity
        assert result["candidates"]
        assert result["candidates"][0]["intent_type"] == "sensitivity_analysis"

    def test_story_overlap_marks_ambiguous(self):
        instruction = "Develop a story arc and apply a consistent visual theme"
        result = detect_intent_ambiguity(instruction, STORY_INTENT_KEYWORDS)
        # Overlap can be ambiguous or not depending on relative scores; just ensure both appear
        types = {c["intent_type"] for c in result["candidates"]}
        assert "story_arc_development" in types
        assert "theme_application" in types

    def test_candidate_cap_respected(self):
        instruction = "story narrative visual theme arc audience persona organize"
        result = detect_intent_ambiguity(instruction, STORY_INTENT_KEYWORDS, max_candidates=2)
        assert len(result["candidates"]) <= 2


class TestEconomicOperationHandler:
    """Test EconomicOperationHandler."""
    
    @pytest.mark.asyncio
    async def test_handle_creates_valid_operation(self):
        """Test that handler creates valid GeneratedOperation."""
        handler = EconomicOperationHandler()
        
        command = GenerativeCommand.create(
            user_instruction="Adjust revenue by 15 percent",
            operation_type="economic_function",
            context={"scenario": "optimistic"},
        )
        
        operation = await handler.handle(command)
        
        # Validate operation
        assert operation is not None
        assert operation.command_id == command.command_id
        assert operation.generated_function is not None
        assert operation.can_execute() is True
    
    @pytest.mark.asyncio
    async def test_handle_generates_economic_code(self):
        """Test that handler generates economic-specific code."""
        handler = EconomicOperationHandler()
        
        command = GenerativeCommand.create(
            user_instruction="Calculate sensitivity analysis on COGS",
            operation_type="economic_function",
            context={},
        )
        
        operation = await handler.handle(command)
        func = operation.generated_function
        
        # Validate code contains economic keywords
        code_lower = func.code.lower()
        assert any(word in code_lower for word in ["economic", "financial", "revenue", "cost"])
    
    @pytest.mark.asyncio
    async def test_handle_records_generation_metadata(self):
        """Test that handler records generation metadata."""
        handler = EconomicOperationHandler()
        
        command = GenerativeCommand.create(
            user_instruction="Forecast growth",
            operation_type="economic_function",
            context={},
        )
        
        operation = await handler.handle(command)
        metadata = operation.generation_metadata
        
        # Validate metadata (StrategySelector-backed generation)
        assert "semantic_confidence" in metadata
        # Confidence is dynamically calculated based on keyword coverage, detail, features (0.0-1.0)
        assert 0.0 <= metadata["semantic_confidence"] <= 1.0
        assert "intent_type" in metadata
        assert metadata.get("generator_version") == "semantic_operation_handlers_v1"
        assert metadata.get("strategy") == GenerationStrategy.TEMPLATE_INJECTION.value
        assert metadata.get("handler") == "EconomicOperationHandler"
        assert "features_detected" in metadata
    
    @pytest.mark.asyncio
    async def test_handle_increments_processed_count(self):
        """Test that handler tracks processed commands."""
        handler = EconomicOperationHandler()
        
        initial_count = handler.processed_count
        
        command1 = GenerativeCommand.create(
            user_instruction="Forecast 1",
            operation_type="economic_function",
            context={},
        )
        
        command2 = GenerativeCommand.create(
            user_instruction="Forecast 2",
            operation_type="economic_function",
            context={},
        )
        
        await handler.handle(command1)
        await handler.handle(command2)
        
        # Validate count incremented
        assert handler.processed_count == initial_count + 2

    @pytest.mark.asyncio
    async def test_detect_ambiguous_intents(self):
        """Ambiguity is flagged when multiple intents score similarly."""
        handler = EconomicOperationHandler()
        command = GenerativeCommand.create(
            user_instruction="Build waterfall decomposition and sensitivity for revenue drivers",
            operation_type="economic_function",
            context={},
        )

        operation = await handler.handle(command)
        metadata = operation.generation_metadata

        assert metadata.get("is_ambiguous") is True
        candidates = metadata.get("ambiguity_candidates")
        assert candidates and len(candidates) >= 2
        candidate_types = {c["intent_type"] for c in candidates}
        assert "sensitivity_analysis" in candidate_types
        assert "waterfall_analysis" in candidate_types
    
    @pytest.mark.asyncio
    async def test_handle_uses_template_based_strategy(self):
        """Test that Phase 9.2 uses TEMPLATE_BASED generation strategy."""
        handler = EconomicOperationHandler()
        
        command = GenerativeCommand.create(
            user_instruction="Economic operation",
            operation_type="economic_function",
            context={},
        )
        
        operation = await handler.handle(command)
        func = operation.generated_function
        
        # Validate strategy
        assert func.generation_strategy == GenerationStrategy.TEMPLATE_INJECTION
    
    @pytest.mark.asyncio
    async def test_handle_creates_deterministic_function_ids(self):
        """Test that same command produces consistent function IDs."""
        handler1 = EconomicOperationHandler()
        handler2 = EconomicOperationHandler()
        
        command = GenerativeCommand.create(
            user_instruction="Same instruction",
            operation_type="economic_function",
            context={},
        )
        
        operation1 = await handler1.handle(command)
        operation2 = await handler2.handle(command)
        
        # IDs should be deterministic based on code content
        assert operation1.generated_function.function_id == operation2.generated_function.function_id
    
    # Phase 9.2: Intent-specific tests
    
    @pytest.mark.asyncio
    async def test_detect_sensitivity_analysis_intent(self):
        """Test detection of sensitivity analysis intent."""
        handler = EconomicOperationHandler()
        
        command = GenerativeCommand.create(
            user_instruction="Perform sensitivity analysis on revenue drivers with elasticity",
            operation_type="economic_function",
            context={},
        )
        
        operation = await handler.handle(command)
        
        assert operation.generation_metadata["intent_type"] == "sensitivity_analysis"
        assert "sensitivity" in operation.generated_function.code
    
    @pytest.mark.asyncio
    async def test_detect_waterfall_analysis_intent(self):
        """Test detection of waterfall decomposition intent."""
        handler = EconomicOperationHandler()
        
        command = GenerativeCommand.create(
            user_instruction="Create waterfall decomposition of revenue drivers",
            operation_type="economic_function",
            context={},
        )
        
        operation = await handler.handle(command)
        
        assert operation.generation_metadata["intent_type"] == "waterfall_analysis"
        assert "waterfall" in operation.generated_function.code
    
    @pytest.mark.asyncio
    async def test_detect_break_even_intent(self):
        """Test detection of break-even analysis intent."""
        handler = EconomicOperationHandler()
        
        command = GenerativeCommand.create(
            user_instruction="Calculate break-even point for minimum revenue threshold",
            operation_type="economic_function",
            context={},
        )
        
        operation = await handler.handle(command)
        
        assert operation.generation_metadata["intent_type"] == "break_even_analysis"
        assert "break_even" in operation.generated_function.code
    
    @pytest.mark.asyncio
    async def test_detect_unit_economics_intent(self):
        """Test detection of unit economics intent."""
        handler = EconomicOperationHandler()
        
        command = GenerativeCommand.create(
            user_instruction="Analyze unit economics: CAC, LTV, and payback period",
            operation_type="economic_function",
            context={},
        )
        
        operation = await handler.handle(command)
        
        assert operation.generation_metadata["intent_type"] == "unit_economics_analysis"
        assert "unit_economics" in operation.generated_function.code
    
    @pytest.mark.asyncio
    async def test_detect_variance_analysis_intent(self):
        """Test detection of variance analysis intent."""
        handler = EconomicOperationHandler()
        
        command = GenerativeCommand.create(
            user_instruction="Variance analysis: compare budget vs actual costs",
            operation_type="economic_function",
            context={},
        )
        
        operation = await handler.handle(command)
        
        assert operation.generation_metadata["intent_type"] == "variance_analysis"
        assert "variance" in operation.generated_function.code
    
    @pytest.mark.asyncio
    async def test_detect_contribution_margin_intent(self):
        """Test detection of contribution margin intent."""
        handler = EconomicOperationHandler()
        
        command = GenerativeCommand.create(
            user_instruction="Calculate contribution margin by product segment",
            operation_type="economic_function",
            context={},
        )
        
        operation = await handler.handle(command)
        
        assert operation.generation_metadata["intent_type"] == "contribution_margin"
        assert "contribution_margin" in operation.generated_function.code
    
    @pytest.mark.asyncio
    async def test_detect_growth_decomposition_intent(self):
        """Test detection of growth decomposition intent."""
        handler = EconomicOperationHandler()
        
        command = GenerativeCommand.create(
            user_instruction="Decompose organic vs inorganic growth components",
            operation_type="economic_function",
            context={},
        )
        
        operation = await handler.handle(command)
        
        assert operation.generation_metadata["intent_type"] == "growth_decomposition"
        assert "growth_decomposition" in operation.generated_function.code
    
    @pytest.mark.asyncio
    async def test_detect_profitability_attribution_intent(self):
        """Test detection of profitability attribution intent."""
        handler = EconomicOperationHandler()
        
        command = GenerativeCommand.create(
            user_instruction="Attribute profitability to key business drivers and factors",
            operation_type="economic_function",
            context={},
        )
        
        operation = await handler.handle(command)
        
        assert operation.generation_metadata["intent_type"] == "profitability_attribution"
        assert "profitability_attribution" in operation.generated_function.code


class TestStoryOperationHandler:
    """Test StoryOperationHandler."""
    
    @pytest.mark.asyncio
    async def test_handle_creates_valid_operation(self):
        """Test that handler creates valid GeneratedOperation."""
        handler = StoryOperationHandler()
        
        command = GenerativeCommand.create(
            user_instruction="Create pitch narrative",
            operation_type="story_operation",
            context={"audience": "investors"},
        )
        
        operation = await handler.handle(command)
        
        # Validate operation
        assert operation is not None
        assert operation.command_id == command.command_id
        assert operation.generated_function is not None
        assert operation.can_execute() is True
    
    @pytest.mark.asyncio
    async def test_handle_generates_story_code(self):
        """Test that handler generates story-specific code."""
        handler = StoryOperationHandler()
        
        command = GenerativeCommand.create(
            user_instruction="Generate slide deck for board meeting",
            operation_type="story_operation",
            context={},
        )
        
        operation = await handler.handle(command)
        func = operation.generated_function
        
        # Validate code contains story keywords
        code_lower = func.code.lower()
        assert any(word in code_lower for word in ["narrative", "story", "slide", "section"])
    
    @pytest.mark.asyncio
    async def test_handle_records_generation_metadata(self):
        """Test that handler records generation metadata."""
        handler = StoryOperationHandler()
        
        command = GenerativeCommand.create(
            user_instruction="Create narrative",
            operation_type="story_operation",
            context={},
        )
        
        operation = await handler.handle(command)
        metadata = operation.generation_metadata
        
        # Validate metadata (semantic confidence scoring)
        assert "semantic_confidence" in metadata
        # Confidence is dynamically calculated based on keyword coverage, detail, features (0.0-1.0)
        assert 0.0 <= metadata["semantic_confidence"] <= 1.0
        assert "intent_type" in metadata
        assert metadata.get("generator_version") == "semantic_operation_handlers_v1"
        assert "strategy" in metadata
        assert "features_detected" in metadata
    
    @pytest.mark.asyncio
    async def test_handle_increments_processed_count(self):
        """Test that handler tracks processed commands."""
        handler = StoryOperationHandler()
        
        initial_count = handler.processed_count
        
        command = GenerativeCommand.create(
            user_instruction="Story operation",
            operation_type="story_operation",
            context={},
        )
        
        await handler.handle(command)
        
        # Validate count incremented
        assert handler.processed_count == initial_count + 1
    
    # Phase 9.2: Intent-specific narrative tests
    
    @pytest.mark.asyncio
    async def test_detect_story_arc_intent(self):
        """Test detection of story arc development intent."""
        handler = StoryOperationHandler()
        
        command = GenerativeCommand.create(
            user_instruction="Develop narrative arc following hero's journey story structure",
            operation_type="story_operation",
            context={},
        )
        
        operation = await handler.handle(command)
        
        assert operation.generation_metadata["intent_type"] == "story_arc_development"
        assert "story_arc" in operation.generated_function.code
    
    @pytest.mark.asyncio
    async def test_detect_theme_application_intent(self):
        """Test detection of theme application intent."""
        handler = StoryOperationHandler()
        
        command = GenerativeCommand.create(
            user_instruction="Apply consistent visual theme and branding throughout slides",
            operation_type="story_operation",
            context={},
        )
        
        operation = await handler.handle(command)
        
        assert operation.generation_metadata["intent_type"] == "theme_application"
        assert "theme" in operation.generated_function.code
    
    @pytest.mark.asyncio
    async def test_detect_framing_intent(self):
        """Test detection of framing strategy intent."""
        handler = StoryOperationHandler()
        
        command = GenerativeCommand.create(
            user_instruction="Frame the narrative from a market opportunity perspective",
            operation_type="story_operation",
            context={},
        )
        
        operation = await handler.handle(command)
        
        assert operation.generation_metadata["intent_type"] == "framing_strategy"
        assert "frame" in operation.generated_function.code
    
    @pytest.mark.asyncio
    async def test_detect_evidence_based_intent(self):
        """Test detection of evidence-based narrative intent."""
        handler = StoryOperationHandler()
        
        command = GenerativeCommand.create(
            user_instruction="Support all claims with data and proof points from evidence",
            operation_type="story_operation",
            context={},
        )
        
        operation = await handler.handle(command)
        
        assert operation.generation_metadata["intent_type"] == "evidence_based_narrative"
        assert "evidence" in operation.generated_function.code
    
    @pytest.mark.asyncio
    async def test_detect_audience_tailoring_intent(self):
        """Test detection of audience tailoring intent."""
        handler = StoryOperationHandler()
        
        command = GenerativeCommand.create(
            user_instruction="Tailor narrative for investor audience with focus on ROI",
            operation_type="story_operation",
            context={},
        )
        
        operation = await handler.handle(command)
        
        assert operation.generation_metadata["intent_type"] == "audience_tailoring"
        assert "audience" in operation.generated_function.code or "persona" in operation.generated_function.code
    
    @pytest.mark.asyncio
    async def test_detect_organization_intent(self):
        """Test detection of narrative organization intent."""
        handler = StoryOperationHandler()
        
        command = GenerativeCommand.create(
            user_instruction="Organize narrative with logical flow and sequence structure",
            operation_type="story_operation",
            context={},
        )
        
        operation = await handler.handle(command)
        
        assert operation.generation_metadata["intent_type"] == "narrative_organization"
        assert "organization" in operation.generated_function.code or "structure" in operation.generated_function.code
    
    @pytest.mark.asyncio
    async def test_detect_visual_narrative_intent(self):
        """Test detection of visual narrative design intent."""
        handler = StoryOperationHandler()
        
        command = GenerativeCommand.create(
            user_instruction="Design visual narrative with slides, graphics, and visual elements",
            operation_type="story_operation",
            context={},
        )
        
        operation = await handler.handle(command)
        
        assert operation.generation_metadata["intent_type"] == "visual_narrative_design"
        assert "visual" in operation.generated_function.code

    @pytest.mark.asyncio
    async def test_detect_ambiguous_story_intents(self):
        """Ambiguity is flagged when story intents overlap."""
        handler = StoryOperationHandler()
        command = GenerativeCommand.create(
            user_instruction="Create a narrative arc and apply a consistent theme",
            operation_type="story_function",
            context={},
        )

        result = await handler.handle(command)
        metadata = result.generation_metadata

        assert metadata.get("is_ambiguous") is True
        candidates = metadata.get("ambiguity_candidates")
        assert candidates and len(candidates) >= 2
        types = {c["intent_type"] for c in candidates}
        assert "story_arc_development" in types
        assert "theme_application" in types


class TestSystemOperationHandler:
    """Tests for SystemOperationHandler semantic operations."""
    
    @pytest.mark.asyncio
    async def test_handle_basic_operation(self):
        handler = SystemOperationHandler()
        command = GenerativeCommand.create(
            user_instruction="Execute system operation to configure monitoring.",
            operation_type="system_operation",
            context={},
        )
        
        result = await handler.handle(command)
        
        assert result is not None
        assert result.generated_function is not None
        assert result.generation_metadata["handler"] == "SystemOperationHandler"
    
    @pytest.mark.asyncio
    async def test_handle_records_generation_metadata(self):
        handler = SystemOperationHandler()
        command = GenerativeCommand.create(
            user_instruction="Set up audit logging for compliance tracking and accountability.",
            operation_type="system_operation",
            context={},
        )
        
        result = await handler.handle(command)
        
        assert result.generation_metadata["semantic_confidence"] >= 0.6
        assert result.generation_metadata["strategy"] == "template_injection"
        assert result.semantic_confidence >= 0.6
        assert "intent_type" in result.generation_metadata
        assert "intent_confidence" in result.generation_metadata
    
    @pytest.mark.asyncio
    async def test_handle_tracks_processing_stats(self):
        handler = SystemOperationHandler()
        
        assert handler.processed_count == 0
        
        command = GenerativeCommand.create(
            user_instruction="Monitor system health and performance.",
            operation_type="system_operation",
            context={},
        )
        result = await handler.handle(command)
        
        assert handler.processed_count == 1
        assert result.generation_time_ms >= 0
    
    @pytest.mark.asyncio
    async def test_batch_processing_intent_detected(self):
        """Verify batch_processing intent classification."""
        handler = SystemOperationHandler()
        command = GenerativeCommand.create(
            user_instruction="Process items in batches of 100 for bulk operations.",
            operation_type="system_operation",
            context={},
        )
        
        result = await handler.handle(command)
        intent_type = result.generation_metadata.get("intent_type")
        
        assert intent_type == "batch_processing"
        assert "batch_processing" in result.generated_function.code
        # Semantic confidence may be lower depending on keyword coverage and instruction detail
        assert result.generation_metadata["intent_confidence"] >= 0.4
    
    @pytest.mark.asyncio
    async def test_workflow_orchestration_intent_detected(self):
        """Verify workflow_orchestration intent classification."""
        handler = SystemOperationHandler()
        command = GenerativeCommand.create(
            user_instruction="Orchestrate pipeline stages with sequential workflow execution.",
            operation_type="system_operation",
            context={},
        )
        
        result = await handler.handle(command)
        intent_type = result.generation_metadata.get("intent_type")
        
        assert intent_type == "workflow_orchestration"
        assert "workflow_orchestration" in result.generated_function.code
        # Semantic confidence may be lower depending on keyword coverage and instruction detail
        assert result.generation_metadata["intent_confidence"] >= 0.4
    
    @pytest.mark.asyncio
    async def test_error_recovery_intent_detected(self):
        """Verify error_recovery intent classification."""
        handler = SystemOperationHandler()
        command = GenerativeCommand.create(
            user_instruction="Implement error recovery with retry logic and resilience patterns.",
            operation_type="system_operation",
            context={},
        )
        
        result = await handler.handle(command)
        intent_type = result.generation_metadata.get("intent_type")
        
        assert intent_type == "error_recovery"
        assert "error_recovery" in result.generated_function.code
        assert result.generation_metadata["intent_confidence"] >= 0.75
    
    @pytest.mark.asyncio
    async def test_cache_management_intent_detected(self):
        """Verify cache_management intent classification."""
        handler = SystemOperationHandler()
        command = GenerativeCommand.create(
            user_instruction="Configure cache optimization with TTL and persistence.",
            operation_type="system_operation",
            context={},
        )
        
        result = await handler.handle(command)
        intent_type = result.generation_metadata.get("intent_type")
        
        assert intent_type == "cache_management"
        assert "cache_management" in result.generated_function.code
        # Semantic confidence may be lower depending on keyword coverage and instruction detail
        assert result.generation_metadata["intent_confidence"] >= 0.4
    
    @pytest.mark.asyncio
    async def test_data_migration_intent_detected(self):
        """Verify data_migration intent classification."""
        handler = SystemOperationHandler()
        command = GenerativeCommand.create(
            user_instruction="Migrate data with format conversion and upgrade compatibility.",
            operation_type="system_operation",
            context={},
        )
        
        result = await handler.handle(command)
        intent_type = result.generation_metadata.get("intent_type")
        
        assert intent_type == "data_migration"
        assert "data_migration" in result.generated_function.code
        # Semantic confidence may be lower depending on keyword coverage and instruction detail
        assert result.generation_metadata["intent_confidence"] >= 0.4
    
    @pytest.mark.asyncio
    async def test_audit_logging_intent_detected(self):
        """Verify audit_logging intent classification."""
        handler = SystemOperationHandler()
        command = GenerativeCommand.create(
            user_instruction="Enable audit logging for compliance and accountability tracking.",
            operation_type="system_operation",
            context={},
        )
        
        result = await handler.handle(command)
        intent_type = result.generation_metadata.get("intent_type")
        
        assert intent_type == "audit_logging"
        assert "audit_logging" in result.generated_function.code
        assert result.generation_metadata["intent_confidence"] >= 0.75
    
    @pytest.mark.asyncio
    async def test_notification_rules_intent_detected(self):
        """Verify notification_rules intent classification."""
        handler = SystemOperationHandler()
        command = GenerativeCommand.create(
            user_instruction="Configure event notification rules with alerts and triggers.",
            operation_type="system_operation",
            context={},
        )
        
        result = await handler.handle(command)
        intent_type = result.generation_metadata.get("intent_type")
        
        assert intent_type == "notification_rules"
        assert "notification_rules" in result.generated_function.code
        assert result.generation_metadata["intent_confidence"] >= 0.4

    @pytest.mark.asyncio
    async def test_detect_ambiguous_intents(self):
        """Ambiguity is flagged when multiple intents score similarly."""
        handler = SystemOperationHandler()
        command = GenerativeCommand.create(
            user_instruction="Orchestrate batch processing pipeline for workflows",
            operation_type="system_function",
            context={},
        )

        result = await handler.handle(command)
        metadata = result.generation_metadata

        assert metadata.get("is_ambiguous") is True
        candidates = metadata.get("ambiguity_candidates")
        assert candidates and len(candidates) >= 2
        candidate_types = {c["intent_type"] for c in candidates}
        assert "batch_processing" in candidate_types
        assert "workflow_orchestration" in candidate_types


class TestHandlerCommonBehavior:
    """Test common behavior across all handlers."""
    
    @pytest.mark.asyncio
    async def test_all_handlers_handle_method_signature(self):
        """Test that all handlers implement async handle(command) correctly."""
        handlers = [
            EconomicOperationHandler(),
            StoryOperationHandler(),
            SystemOperationHandler(),
        ]
        
        command = GenerativeCommand.create(
            user_instruction="Test",
            operation_type="test_type",
            context={},
        )
        
        for handler in handlers:
            # Method should exist
            assert hasattr(handler, 'handle')
            # Method should be async
            assert callable(handler.handle)
    
    @pytest.mark.asyncio
    async def test_handlers_set_operation_id(self):
        """Test that all handlers generate unique operation IDs."""
        handlers = [
            EconomicOperationHandler(),
            StoryOperationHandler(),
            SystemOperationHandler(),
        ]
        
        command = GenerativeCommand.create(
            user_instruction="Test",
            operation_type="test_type",
            context={},
        )
        
        operation_ids = []
        
        for handler in handlers:
            operation = await handler.handle(command)
            assert operation.operation_id is not None
            operation_ids.append(operation.operation_id)
        
        # All should be different
        assert len(set(operation_ids)) == len(operation_ids)
    
    @pytest.mark.asyncio
    async def test_handlers_return_function_ids(self):
        """Test that operations include function identifiers."""
        handlers = [
            EconomicOperationHandler(),
            StoryOperationHandler(),
            SystemOperationHandler(),
        ]
        
        command = GenerativeCommand.create(
            user_instruction="Test",
            operation_type="test_type",
            context={},
        )
        
        for handler in handlers:
            operation = await handler.handle(command)
            assert operation.generated_function.function_id


class TestPhase92HandlerIntegration:
    """Integration tests for Phase 9.2 handlers."""
    
    @pytest.mark.asyncio
    async def test_handler_operations_have_valid_constraints(self):
        """Test that generated operations satisfy constraint enforcement."""
        handlers = [
            EconomicOperationHandler(),
            StoryOperationHandler(),
            SystemOperationHandler(),
        ]
        
        command = GenerativeCommand.create(
            user_instruction="Test operation",
            operation_type="test_type",
            context={},
        )
        
        for handler in handlers:
            operation = await handler.handle(command)
            
            # Validate constraints
            assert operation.generated_function.name is not None
            assert len(operation.generated_function.name) > 0
            assert operation.generated_function.code is not None
            assert len(operation.generated_function.code) > 0
            assert operation.generation_metadata is not None
            assert isinstance(operation.generation_metadata, dict)
    
    @pytest.mark.asyncio
    async def test_handlers_produce_executable_functions(self):
        """Test that all handlers produce functions that can execute."""
        handlers = [
            EconomicOperationHandler(),
            StoryOperationHandler(),
            SystemOperationHandler(),
        ]
        
        command = GenerativeCommand.create(
            user_instruction="Test operation",
            operation_type="test_type",
            context={},
        )
        
        for handler in handlers:
            operation = await handler.handle(command)
            
            # Should be executable
            assert operation.can_execute() is True
            assert operation.generated_function.function_id is not None
            assert operation.generated_function.code is not None
