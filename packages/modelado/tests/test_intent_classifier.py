"""
Unit tests for intent_classifier.py

Tests cover:
- Intent classification with LLM mocking
- Few-shot example management
- Batch classification
- Confidence thresholding
- Feature detection
"""

import json
from unittest.mock import AsyncMock, MagicMock, patch
import pytest
from types import SimpleNamespace

from modelado.intent_classifier import (
    IntentClassifier,
    IntentClass,
    IntentClassificationResult,
)


class TestIntentClassifier:
    """Test IntentClassifier class."""
    
    @pytest.fixture
    def mock_openai_client(self):
        """Mock unified llm client for testing."""
        client = AsyncMock()
        client.generate.return_value = SimpleNamespace(
            text=json.dumps({
                "predicted_class": "economic_function",
                "confidence": 0.92,
                "reasoning": "Intent involves financial model correlation",
                "detected_features": {
                    "uses_mathematical_model": True,
                    "requires_correlation": True,
                },
            })
        )

        return client
    
    @pytest.fixture
    def intent_classifier(self, mock_openai_client):
        """IntentClassifier instance with mocked client."""
        classifier = IntentClassifier(
            openai_api_key="test-key",
            model="gpt-4o-mini",
            temperature=0.0,
            confidence_threshold=0.7,
            ai_client=mock_openai_client,
        )
        return classifier
    
    @pytest.mark.asyncio
    async def test_classify_economic_function(self, intent_classifier, mock_openai_client):
        """Test classification of economic function intent."""
        intent = "Correlate revenue with market size using sigmoid"
        
        result = await intent_classifier.classify(intent)
        
        # Verify API called with correct parameters
        mock_openai_client.generate.assert_called_once()
        request = mock_openai_client.generate.call_args.args[0]
        assert request.model == "gpt-4o-mini"
        assert request.temperature == 0.0
        assert request.response_format == {"type": "json_object"}
        
        # Verify result
        assert result.intent == intent
        assert result.predicted_class == IntentClass.ECONOMIC_FUNCTION
        assert result.confidence == 0.92
        assert "correlation" in result.reasoning.lower()
        assert result.detected_features["uses_mathematical_model"] is True
        assert result.classification_time_ms > 0
    
    @pytest.mark.asyncio
    async def test_classify_story_operation(self, intent_classifier, mock_openai_client):
        """Test classification of story operation intent."""
        # Mock response for story operation
        mock_openai_client.generate.return_value = SimpleNamespace(text=json.dumps({
            "predicted_class": "story_operation",
            "confidence": 0.88,
            "reasoning": "Intent involves narrative generation",
            "detected_features": {
                "narrative_generation": True,
                "theme_application": True,
            },
        }))
        
        intent = "Generate narrative arc emphasizing unit economics"
        result = await intent_classifier.classify(intent)
        
        assert result.predicted_class == IntentClass.STORY_OPERATION
        assert result.confidence == 0.88
        assert result.detected_features["narrative_generation"] is True
    
    @pytest.mark.asyncio
    async def test_classify_system_operation(self, intent_classifier, mock_openai_client):
        """Test classification of system operation intent."""
        mock_openai_client.generate.return_value = SimpleNamespace(text=json.dumps({
            "predicted_class": "system_operation",
            "confidence": 0.95,
            "reasoning": "Intent involves export operation",
            "detected_features": {
                "export_operation": True,
                "format_conversion": True,
            },
        }))
        
        intent = "Export financial model to Excel"
        result = await intent_classifier.classify(intent)
        
        assert result.predicted_class == IntentClass.SYSTEM_OPERATION
        assert result.confidence == 0.95
        assert result.detected_features["export_operation"] is True
    
    @pytest.mark.asyncio
    async def test_classify_low_confidence_marked_as_unknown(self, intent_classifier, mock_openai_client):
        """Test that low confidence classifications are marked as unknown."""
        # Mock low confidence response
        mock_openai_client.generate.return_value = SimpleNamespace(text=json.dumps({
            "predicted_class": "economic_function",
            "confidence": 0.45,  # Below threshold (0.7)
            "reasoning": "Ambiguous intent",
            "detected_features": {},
        }))
        
        intent = "Do something with data"
        result = await intent_classifier.classify(intent)
        
        # Should be marked as unknown due to low confidence
        assert result.predicted_class == IntentClass.UNKNOWN
        assert result.confidence == 0.45
    
    @pytest.mark.asyncio
    async def test_classify_batch_parallel_execution(self, intent_classifier, mock_openai_client):
        """Test batch classification."""
        intents = [
            "Adjust revenue forecast",
            "Create slide deck",
            "Export to PDF",
        ]
        
        # Mock responses for each intent
        responses = [
            {"predicted_class": "economic_function", "confidence": 0.9, "reasoning": "Revenue", "detected_features": {}},
            {"predicted_class": "story_operation", "confidence": 0.85, "reasoning": "Slides", "detected_features": {}},
            {"predicted_class": "system_operation", "confidence": 0.92, "reasoning": "Export", "detected_features": {}},
        ]
        
        mock_openai_client.generate.side_effect = [
            SimpleNamespace(text=json.dumps(r))
            for r in responses
        ]
        
        results = await intent_classifier.classify_batch(intents)
        
        # Verify all classified
        assert len(results) == 3
        assert results[0].predicted_class == IntentClass.ECONOMIC_FUNCTION
        assert results[1].predicted_class == IntentClass.STORY_OPERATION
        assert results[2].predicted_class == IntentClass.SYSTEM_OPERATION
        
        # Verify parallel execution (all 3 API calls made)
        assert mock_openai_client.generate.call_count == 3
    
    @pytest.mark.asyncio
    async def test_classify_error_handling(self, intent_classifier, mock_openai_client):
        """Test error handling during classification."""
        mock_openai_client.generate.side_effect = Exception("API error")
        
        with pytest.raises(Exception, match="API error"):
            await intent_classifier.classify("test intent")
    
    def test_format_examples(self, intent_classifier):
        """Test few-shot example formatting."""
        formatted = intent_classifier._format_examples()
        
        # Should include all examples
        assert "Correlate revenue with market size" in formatted
        assert "economic_function" in formatted
        assert "Generate a narrative arc" in formatted
        assert "story_operation" in formatted
    
    def test_get_examples_for_class(self, intent_classifier):
        """Test retrieving examples by class."""
        economic_examples = intent_classifier.get_examples_for_class(IntentClass.ECONOMIC_FUNCTION)
        story_examples = intent_classifier.get_examples_for_class(IntentClass.STORY_OPERATION)
        system_examples = intent_classifier.get_examples_for_class(IntentClass.SYSTEM_OPERATION)
        
        # Should have examples for each class
        assert len(economic_examples) > 0
        assert len(story_examples) > 0
        assert len(system_examples) > 0
        
        # Examples should be correct class
        assert all(ex["class"] == "economic_function" for ex in economic_examples)
        assert all(ex["class"] == "story_operation" for ex in story_examples)
        assert all(ex["class"] == "system_operation" for ex in system_examples)
    
    def test_add_example(self, intent_classifier):
        """Test adding new few-shot example."""
        initial_count = len(intent_classifier.FEW_SHOT_EXAMPLES)
        
        intent_classifier.add_example(
            intent="New example intent",
            intent_class=IntentClass.ECONOMIC_FUNCTION,
            features={"new_feature": True},
        )
        
        # Example count should increase
        assert len(intent_classifier.FEW_SHOT_EXAMPLES) == initial_count + 1
        
        # New example should be retrievable
        economic_examples = intent_classifier.get_examples_for_class(IntentClass.ECONOMIC_FUNCTION)
        assert any(ex["intent"] == "New example intent" for ex in economic_examples)
    
    def test_get_stats(self, intent_classifier):
        """Test classifier statistics."""
        stats = intent_classifier.get_stats()
        
        assert "model" in stats
        assert "temperature" in stats
        assert "confidence_threshold" in stats
        assert "total_examples" in stats
        assert "examples_by_class" in stats
        
        assert stats["model"] == "gpt-4o-mini"
        assert stats["temperature"] == 0.0
        assert stats["confidence_threshold"] == 0.7
        assert stats["total_examples"] > 0
        
        # Should have counts for all classes
        assert IntentClass.ECONOMIC_FUNCTION.value in stats["examples_by_class"]
        assert IntentClass.STORY_OPERATION.value in stats["examples_by_class"]
        assert IntentClass.SYSTEM_OPERATION.value in stats["examples_by_class"]


class TestIntentClass:
    """Test IntentClass enum."""
    
    def test_intent_class_values(self):
        """Test IntentClass enum values."""
        assert IntentClass.ECONOMIC_FUNCTION.value == "economic_function"
        assert IntentClass.STORY_OPERATION.value == "story_operation"
        assert IntentClass.SYSTEM_OPERATION.value == "system_operation"
        assert IntentClass.UNKNOWN.value == "unknown"
    
    def test_intent_class_from_string(self):
        """Test creating IntentClass from string."""
        assert IntentClass("economic_function") == IntentClass.ECONOMIC_FUNCTION
        assert IntentClass("story_operation") == IntentClass.STORY_OPERATION
        assert IntentClass("system_operation") == IntentClass.SYSTEM_OPERATION
        assert IntentClass("unknown") == IntentClass.UNKNOWN


class TestIntentClassificationResult:
    """Test IntentClassificationResult dataclass."""
    
    def test_classification_result_creation(self):
        """Test creating IntentClassificationResult."""
        result = IntentClassificationResult(
            intent="test intent",
            predicted_class=IntentClass.ECONOMIC_FUNCTION,
            confidence=0.92,
            reasoning="Test reasoning",
            detected_features={"feature1": True},
            classification_time_ms=25.5,
        )
        
        assert result.intent == "test intent"
        assert result.predicted_class == IntentClass.ECONOMIC_FUNCTION
        assert result.confidence == 0.92
        assert result.reasoning == "Test reasoning"
        assert result.detected_features == {"feature1": True}
        assert result.classification_time_ms == 25.5
