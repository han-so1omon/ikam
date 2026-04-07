"""Tests for generative operation command contracts.

Validates:
- Schema correctness
- Serialization/deserialization
- Confidence calculations
- Type validation
"""

import pytest
from datetime import datetime
from modelado.core.generative_commands import (
    GenerationStrategy,
    GenerativeEconomicCommand,
    GenerativeStoryCommand,
    GeneratedOperationResult,
)


class TestGenerativeEconomicCommand:
    """Tests for GenerativeEconomicCommand."""
    
    def test_create_minimal_command(self):
        """Create command with required fields only."""
        cmd = GenerativeEconomicCommand(
            semantic_intent="adjust_revenue_growth",
            confidence=0.92
        )
        assert cmd.semantic_intent == "adjust_revenue_growth"
        assert cmd.confidence == 0.92
        assert cmd.intent_class == "ECONOMIC_FUNCTION"
        assert cmd.command_id  # Generated
        assert isinstance(cmd.created_at, datetime)
    
    def test_create_full_command(self):
        """Create command with all fields."""
        cmd = GenerativeEconomicCommand(
            semantic_intent="correlate_costs_headcount",
            confidence=0.87,
            extracted_entities={"rate": 0.15, "period": "annual"},
            context_data={"artifact_id": "art-123"},
            generation_strategy=GenerationStrategy.LLM_BASED,
            source_instruction="What's the relationship between cost and headcount?",
            semantic_features=["correlation", "analysis", "cost_driver"],
            parser_confidence=0.89
        )
        assert cmd.semantic_intent == "correlate_costs_headcount"
        assert cmd.confidence == 0.87
        assert cmd.extracted_entities == {"rate": 0.15, "period": "annual"}
        assert cmd.context_data == {"artifact_id": "art-123"}
        assert cmd.generation_strategy == GenerationStrategy.LLM_BASED
        assert cmd.source_instruction == "What's the relationship between cost and headcount?"
        assert cmd.semantic_features == ["correlation", "analysis", "cost_driver"]
        assert cmd.parser_confidence == 0.89
    
    def test_confidence_range(self):
        """Confidence must be between 0.0 and 1.0."""
        # Valid: 0.0
        cmd1 = GenerativeEconomicCommand(semantic_intent="test", confidence=0.0)
        assert cmd1.confidence == 0.0
        
        # Valid: 1.0
        cmd2 = GenerativeEconomicCommand(semantic_intent="test", confidence=1.0)
        assert cmd2.confidence == 1.0
        
        # Valid: 0.5
        cmd3 = GenerativeEconomicCommand(semantic_intent="test", confidence=0.5)
        assert cmd3.confidence == 0.5
    
    def test_to_dict_serialization(self):
        """Serialize to dictionary."""
        cmd = GenerativeEconomicCommand(
            semantic_intent="test_intent",
            confidence=0.92,
            extracted_entities={"key": "value"},
            parser_confidence=0.88
        )
        d = cmd.to_dict()
        
        assert d["semantic_intent"] == "test_intent"
        assert d["confidence"] == 0.92
        assert d["extracted_entities"] == {"key": "value"}
        assert d["parser_confidence"] == 0.88
        assert d["intent_class"] == "ECONOMIC_FUNCTION"
        assert d["generation_strategy"] == "llm_based"
        assert "command_id" in d
        assert "created_at" in d
    
    def test_from_dict_deserialization(self):
        """Deserialize from dictionary."""
        original = GenerativeEconomicCommand(
            semantic_intent="deserialize_test",
            confidence=0.85,
            extracted_entities={"param": 123}
        )
        d = original.to_dict()
        
        restored = GenerativeEconomicCommand.from_dict(d)
        assert restored.semantic_intent == original.semantic_intent
        assert restored.confidence == original.confidence
        assert restored.extracted_entities == original.extracted_entities
        assert restored.command_id == original.command_id
        assert restored.created_at.isoformat() == original.created_at.isoformat()
    
    def test_combined_confidence_no_parser(self):
        """Combined confidence uses semantic only if parser unavailable."""
        cmd = GenerativeEconomicCommand(
            semantic_intent="test",
            confidence=0.90,
            parser_confidence=None
        )
        assert cmd.combined_confidence() == 0.90
    
    def test_combined_confidence_with_parser(self):
        """Combined confidence blends semantic and parser scores.
        
        Formula: 0.6 * semantic + 0.4 * parser
        """
        cmd = GenerativeEconomicCommand(
            semantic_intent="test",
            confidence=0.90,
            parser_confidence=0.80
        )
        expected = 0.6 * 0.90 + 0.4 * 0.80  # 0.54 + 0.32 = 0.86
        assert cmd.combined_confidence() == pytest.approx(0.86)
    
    def test_intent_class_immutable(self):
        """Intent class is always ECONOMIC_FUNCTION."""
        cmd = GenerativeEconomicCommand(semantic_intent="test", confidence=0.85)
        assert cmd.intent_class == "ECONOMIC_FUNCTION"


class TestGenerativeStoryCommand:
    """Tests for GenerativeStoryCommand."""
    
    def test_create_minimal_command(self):
        """Create story command with required fields."""
        cmd = GenerativeStoryCommand(
            semantic_intent="create_pitch_deck_growth",
            confidence=0.88
        )
        assert cmd.semantic_intent == "create_pitch_deck_growth"
        assert cmd.confidence == 0.88
        assert cmd.intent_class == "STORY_OPERATION"
    
    def test_create_full_command(self):
        """Create story command with all fields."""
        cmd = GenerativeStoryCommand(
            semantic_intent="three_act_narrative_unit_econ",
            confidence=0.91,
            extracted_entities={"tone": "investor", "focus": "growth"},
            context_data={"artifact_id": "slide-456", "num_slides": 15},
            generation_strategy=GenerationStrategy.TEMPLATE,
            source_instruction="Create a three-act story about our unit economics improvement",
            semantic_features=["narrative", "slides", "structure"],
            parser_confidence=0.87
        )
        assert cmd.semantic_intent == "three_act_narrative_unit_econ"
        assert cmd.confidence == 0.91
        assert cmd.extracted_entities["tone"] == "investor"
        assert cmd.intent_class == "STORY_OPERATION"
    
    def test_to_dict_serialization(self):
        """Serialize story command to dictionary."""
        cmd = GenerativeStoryCommand(
            semantic_intent="test_story",
            confidence=0.85,
            extracted_entities={"audience": "investor"}
        )
        d = cmd.to_dict()
        
        assert d["semantic_intent"] == "test_story"
        assert d["intent_class"] == "STORY_OPERATION"
        assert d["generation_strategy"] == "llm_based"
    
    def test_from_dict_deserialization(self):
        """Deserialize story command from dictionary."""
        original = GenerativeStoryCommand(
            semantic_intent="story_test",
            confidence=0.80
        )
        d = original.to_dict()
        
        restored = GenerativeStoryCommand.from_dict(d)
        assert restored.semantic_intent == original.semantic_intent
        assert restored.confidence == original.confidence
    
    def test_combined_confidence_with_parser(self):
        """Story command combined confidence blends scores."""
        cmd = GenerativeStoryCommand(
            semantic_intent="test",
            confidence=0.92,
            parser_confidence=0.85
        )
        expected = 0.6 * 0.92 + 0.4 * 0.85  # 0.552 + 0.34 = 0.892
        assert cmd.combined_confidence() == pytest.approx(0.892)
    
    def test_intent_class_immutable(self):
        """Intent class is always STORY_OPERATION."""
        cmd = GenerativeStoryCommand(semantic_intent="test", confidence=0.90)
        assert cmd.intent_class == "STORY_OPERATION"


class TestGeneratedOperationResult:
    """Tests for GeneratedOperationResult."""
    
    def test_create_success_result(self):
        """Create successful operation result."""
        result = GeneratedOperationResult(
            command_id="cmd-123",
            semantic_intent="adjust_revenue",
            generated_function_id="func-abc",
            execution_result={"new_revenue": 1500000},
            confidence_scores={"combined": 0.92},
            execution_status="success"
        )
        assert result.command_id == "cmd-123"
        assert result.semantic_intent == "adjust_revenue"
        assert result.execution_status == "success"
        assert result.error is None
    
    def test_create_failed_result(self):
        """Create failed operation result."""
        result = GeneratedOperationResult(
            command_id="cmd-456",
            semantic_intent="bad_intent",
            generated_function_id="func-xyz",
            execution_status="failed",
            error="Invalid parameter: rate must be > 0"
        )
        assert result.execution_status == "failed"
        assert result.error == "Invalid parameter: rate must be > 0"
    
    def test_with_full_metadata(self):
        """Create result with all metadata."""
        result = GeneratedOperationResult(
            command_id="cmd-789",
            semantic_intent="correlate_test",
            generated_function_id="func-meta",
            execution_result={"correlation": 0.78},
            generation_metadata={
                "llm_tokens": 450,
                "temperature": 0.7,
                "model": "gpt-4o-mini",
                "seed": 42
            },
            confidence_scores={
                "semantic": 0.88,
                "parser": 0.85,
                "combined": 0.87,
                "execution_confidence": 0.95
            },
            ikam_provenance={
                "artifact_id": "art-123",
                "derivation_type": "generative_operation",
                "generation_strategy": "llm_based",
                "lossless_reconstruction": True
            },
            execution_latency_ms=325.5,
            execution_status="success"
        )
        assert result.generation_metadata["model"] == "gpt-4o-mini"
        assert result.confidence_scores["combined"] == 0.87
        assert result.ikam_provenance["lossless_reconstruction"] is True
        assert result.execution_latency_ms == 325.5
    
    def test_to_dict_serialization(self):
        """Serialize result to dictionary."""
        result = GeneratedOperationResult(
            command_id="cmd-ser",
            semantic_intent="serialize_test",
            generated_function_id="func-ser",
            execution_result={"test": True},
            execution_status="success"
        )
        d = result.to_dict()
        
        assert d["command_id"] == "cmd-ser"
        assert d["execution_status"] == "success"
        assert "result_id" in d
        assert "created_at" in d
    
    def test_from_dict_deserialization(self):
        """Deserialize result from dictionary."""
        original = GeneratedOperationResult(
            command_id="cmd-deser",
            semantic_intent="deser_test",
            generated_function_id="func-deser",
            execution_result={"data": 123}
        )
        d = original.to_dict()
        
        restored = GeneratedOperationResult.from_dict(d)
        assert restored.command_id == original.command_id
        assert restored.semantic_intent == original.semantic_intent
        assert restored.execution_result == original.execution_result


class TestGenerationStrategy:
    """Tests for GenerationStrategy enum."""
    
    def test_all_strategies(self):
        """Verify all strategy values."""
        assert GenerationStrategy.LLM_BASED.value == "llm_based"
        assert GenerationStrategy.COMPOSABLE.value == "composable_building_blocks"
        assert GenerationStrategy.TEMPLATE.value == "template_injection"
        assert GenerationStrategy.UNKNOWN.value == "unknown"
    
    def test_from_string(self):
        """Create strategy from string."""
        strategy = GenerationStrategy("llm_based")
        assert strategy == GenerationStrategy.LLM_BASED


class TestCrossCommandCompatibility:
    """Tests for compatibility between command types."""
    
    def test_economic_and_story_separate(self):
        """Economic and story commands are distinct."""
        econ = GenerativeEconomicCommand(
            semantic_intent="revenue_test",
            confidence=0.85
        )
        story = GenerativeStoryCommand(
            semantic_intent="story_test",
            confidence=0.85
        )
        assert econ.intent_class != story.intent_class
        assert econ.intent_class == "ECONOMIC_FUNCTION"
        assert story.intent_class == "STORY_OPERATION"
    
    def test_confidence_calculation_consistency(self):
        """Both command types use same confidence blending."""
        parser_conf = 0.82
        semantic_conf = 0.89
        
        econ = GenerativeEconomicCommand(
            semantic_intent="test",
            confidence=semantic_conf,
            parser_confidence=parser_conf
        )
        story = GenerativeStoryCommand(
            semantic_intent="test",
            confidence=semantic_conf,
            parser_confidence=parser_conf
        )
        
        expected = 0.6 * semantic_conf + 0.4 * parser_conf
        assert econ.combined_confidence() == pytest.approx(expected)
        assert story.combined_confidence() == pytest.approx(expected)
