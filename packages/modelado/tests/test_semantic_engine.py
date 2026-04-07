"""
Unit tests for semantic_engine.py

Tests cover:
- Intent evaluation with evaluator selection
- Confidence thresholding and fallback logic
- Similar intent finding
- Capability reporting
- Startup validation
- Runtime learning (adding examples)
"""

import pytest
from unittest.mock import AsyncMock, MagicMock, patch

from modelado.semantic_engine import SemanticEngine, SemanticEvaluationResult
from modelado.intent_classifier import IntentClass, IntentClassificationResult
from modelado.semantic_evaluators import EvaluationResult, EvaluatorRegistry
from modelado.semantic_embeddings import EmbeddingResult, SimilarityResult


class TestSemanticEngine:
    """Test SemanticEngine core functionality."""
    
    @pytest.fixture
    def mock_intent_classifier(self):
        """Mock IntentClassifier."""
        classifier = AsyncMock()
        classifier.examples = [{"intent": "test", "class": "economic_function"}]
        return classifier
    
    @pytest.fixture
    def mock_evaluator_registry(self):
        """Mock EvaluatorRegistry."""
        # Note: EvaluatorRegistry has BOTH sync and async methods:
        # - get_all() is sync
        # - get_capabilities() is sync
        # - evaluate_all() is async
        # Use AsyncMock for async methods, but override sync methods
        registry = AsyncMock()
        registry.get_all = MagicMock(return_value=[MagicMock(), MagicMock()])
        registry.get_capabilities = MagicMock(return_value={"total_count": 2})
        return registry

    
    @pytest.fixture
    def mock_embeddings(self):
        """Mock SemanticEmbeddings."""
        embeddings = AsyncMock()
        embeddings.cache = {}
        return embeddings
    
    @pytest.fixture
    def engine(self, mock_intent_classifier, mock_evaluator_registry, mock_embeddings):
        """Create SemanticEngine with mocks."""
        return SemanticEngine(
            intent_classifier=mock_intent_classifier,
            evaluator_registry=mock_evaluator_registry,
            embeddings=mock_embeddings,
            min_confidence=0.5,
        )
    
    @pytest.mark.asyncio
    async def test_evaluate_with_high_confidence_match(self, engine, mock_intent_classifier, mock_evaluator_registry):
        """Test evaluation with evaluator that exceeds confidence threshold."""
        # Setup mocks
        mock_intent_classifier.classify.return_value = IntentClassificationResult(
            intent="Adjust Q4 revenue forecast by 15%",
            predicted_class=IntentClass.ECONOMIC_FUNCTION,
            confidence=0.95,
            reasoning="Clear economic intent",
            detected_features={"revenue": True, "forecast": True},
            classification_time_ms=50.0,
        )
        
        high_confidence_eval = EvaluationResult(
            can_handle=True,
            confidence=0.92,
            reasoning="Strong economic signal",
            capability_metadata={"operations": ["adjust_revenue"]},
            semantic_features={"involves_revenue": True},
            evaluator_name="EconomicFunctionEvaluator",
        )
        
        mock_evaluator_registry.evaluate_all.return_value = [
            high_confidence_eval,
            EvaluationResult(
                can_handle=False,
                confidence=0.2,
                reasoning="Not a story",
                capability_metadata={},
                semantic_features={},
                evaluator_name="StoryOperationEvaluator",
            ),
        ]
        
        # Execute
        result = await engine.evaluate("Adjust Q4 revenue forecast by 15%")
        
        # Verify
        assert result.can_handle is True
        assert result.evaluator_name == "EconomicFunctionEvaluator"
        assert result.evaluator_confidence == 0.92
        assert result.used_generation is False
        assert result.semantic_features["involves_revenue"] is True
        assert len(result.all_evaluations) == 2
    
    @pytest.mark.asyncio
    async def test_evaluate_with_low_confidence_fallback(self, engine, mock_intent_classifier, mock_evaluator_registry):
        """Test evaluation when no evaluator meets confidence threshold."""
        # Setup mocks
        mock_intent_classifier.classify.return_value = IntentClassificationResult(
            intent="Some ambiguous request",
            predicted_class=IntentClass.UNKNOWN,
            confidence=0.3,
            reasoning="Unclear intent",
            detected_features={},
            classification_time_ms=50.0,
        )
        
        low_confidence_eval = EvaluationResult(
            can_handle=True,
            confidence=0.35,  # Below threshold of 0.5
            reasoning="Weak signal",
            capability_metadata={},
            semantic_features={},
            evaluator_name="CustomizationEvaluator",
        )
        
        mock_evaluator_registry.evaluate_all.return_value = [low_confidence_eval]
        
        # Execute
        result = await engine.evaluate("Some ambiguous request")
        
        # Verify - low confidence triggers generative operation creation
        assert result.can_handle is False
        assert result.evaluator_name is None
        assert result.used_generation is True
        assert "below threshold" in result.generation_reason.lower()
    
    @pytest.mark.asyncio
    async def test_evaluate_with_no_capable_evaluators(self, engine, mock_intent_classifier, mock_evaluator_registry):
        """Test evaluation when no evaluator can handle the intent."""
        # Setup mocks
        mock_intent_classifier.classify.return_value = IntentClassificationResult(
            intent="Novel operation never seen before",
            predicted_class=IntentClass.SYSTEM_OPERATION,
            confidence=0.8,
            reasoning="System operation",
            detected_features={"export": True},
            classification_time_ms=50.0,
        )
        
        mock_evaluator_registry.evaluate_all.return_value = [
            EvaluationResult(
                can_handle=False,
                confidence=0.1,
                reasoning="Not economic",
                capability_metadata={},
                semantic_features={},
                evaluator_name="EconomicFunctionEvaluator",
            ),
            EvaluationResult(
                can_handle=False,
                confidence=0.05,
                reasoning="Not a story",
                capability_metadata={},
                semantic_features={},
                evaluator_name="StoryOperationEvaluator",
            ),
        ]
        
        # Execute
        result = await engine.evaluate("Novel operation never seen before")
        
        # Verify - no capable evaluators triggers generative operation creation
        assert result.can_handle is False
        assert result.evaluator_name is None
        assert result.used_generation is True
        assert "generate novel operation" in result.generation_reason.lower()
    
    @pytest.mark.asyncio
    async def test_find_similar_intents(self, engine, mock_embeddings):
        """Test finding similar intents using embeddings."""
        # Setup mock
        mock_embeddings.find_most_similar.return_value = [
            ("Adjust revenue forecast", 0.92),
            ("Increase revenue projection", 0.85),
        ]
        
        # Execute
        candidates = ["Adjust revenue forecast", "Create slide deck", "Increase revenue projection"]
        similar = await engine.find_similar_intents(
            intent="Update revenue forecast",
            candidates=candidates,
            top_k=2,
            threshold=0.7,
        )
        
        # Verify
        assert len(similar) == 2
        assert similar[0][0] == "Adjust revenue forecast"
        assert similar[0][1] == 0.92
        assert similar[1][0] == "Increase revenue projection"
        assert similar[1][1] == 0.85
    
    def test_get_capabilities(self, engine):
        """Test getting engine capabilities."""
        # Setup mock
        engine.evaluator_registry.get_capabilities.return_value = {
            "total_count": 5,
            "evaluators": [
                {"name": "EconomicFunctionEvaluator"},
                {"name": "StoryOperationEvaluator"},
            ],
        }
        
        # Execute
        caps = engine.get_capabilities()
        
        # Verify
        assert caps["total_count"] == 5
        assert len(caps["evaluators"]) == 2
        assert "engine_config" in caps
        assert caps["engine_config"]["min_confidence"] == 0.5
        assert caps["engine_config"]["generative_operations"] == "always_enabled"
    
    def test_add_intent_example(self, engine, mock_intent_classifier):
        """Test adding runtime intent example."""
        # Execute
        engine.add_intent_example(
            intent="Calculate ROI for marketing campaign",
            intent_class=IntentClass.ECONOMIC_FUNCTION,
            features=["calculation", "roi"],
        )
        
        # Verify
        mock_intent_classifier.add_example.assert_called_once_with(
            "Calculate ROI for marketing campaign",
            IntentClass.ECONOMIC_FUNCTION,
            ["calculation", "roi"],
        )
    
    @pytest.mark.asyncio
    async def test_validate_startup_success(self, engine, mock_embeddings):
        """Test successful startup validation."""
        # Setup mock
        from datetime import datetime
        mock_embeddings.embed.return_value = EmbeddingResult(
            text="test",
            embedding=[0.1, 0.2, 0.3],
            model="text-embedding-3-small",
            embedding_id="test-id",
            cached=False,
            generation_time_ms=50.0,
            timestamp=datetime.now(),
        )
        
        # Execute
        is_valid, errors = await engine.validate_startup()
        
        # Verify
        assert is_valid is True
        assert len(errors) == 0
    
    @pytest.mark.asyncio
    async def test_validate_startup_no_examples(self, mock_evaluator_registry, mock_embeddings):
        """Test startup validation fails with no intent examples."""
        # Create classifier with no examples
        classifier = AsyncMock()
        classifier.examples = []
        
        engine = SemanticEngine(
            intent_classifier=classifier,
            evaluator_registry=mock_evaluator_registry,
            embeddings=mock_embeddings,
        )
        
        # Execute
        is_valid, errors = await engine.validate_startup()
        
        # Verify
        assert is_valid is False
        assert any("no few-shot examples" in e.lower() for e in errors)
    
    @pytest.mark.asyncio
    async def test_validate_startup_no_evaluators(self, mock_intent_classifier, mock_embeddings):
        """Test startup validation fails with no evaluators."""
        # Create registry with no evaluators
        registry = MagicMock()
        registry.get_all.return_value = []
        
        engine = SemanticEngine(
            intent_classifier=mock_intent_classifier,
            evaluator_registry=registry,
            embeddings=mock_embeddings,
        )
        
        # Execute
        is_valid, errors = await engine.validate_startup()
        
        # Verify
        assert is_valid is False
        assert any("no evaluators registered" in e.lower() for e in errors)
    
    @pytest.mark.asyncio
    async def test_validate_startup_embeddings_failed(self, mock_intent_classifier, mock_evaluator_registry):
        """Test startup validation fails when embeddings fail."""
        # Create embeddings that fail
        embeddings = AsyncMock()
        embeddings.embed.side_effect = Exception("OpenAI API error")
        
        engine = SemanticEngine(
            intent_classifier=mock_intent_classifier,
            evaluator_registry=mock_evaluator_registry,
            embeddings=embeddings,
        )
        
        # Execute
        is_valid, errors = await engine.validate_startup()
        
        # Verify
        assert is_valid is False
        assert any("semanticembeddings failed" in e.lower() for e in errors)


class TestSemanticEvaluationResult:
    """Test SemanticEvaluationResult dataclass."""
    
    def test_evaluation_result_creation(self):
        """Test creating SemanticEvaluationResult."""
        result = SemanticEvaluationResult(
            intent="Test intent",
            intent_class=IntentClass.ECONOMIC_FUNCTION,
            intent_confidence=0.95,
            intent_features=["revenue", "forecast"],
            evaluator_name="EconomicFunctionEvaluator",
            evaluator_confidence=0.88,
            can_handle=True,
            reasoning="Strong economic signal",
            all_evaluations=[],
            semantic_features={"involves_revenue": True},
            capability_metadata={"operations": ["adjust_revenue"]},
            used_generation=False,
            generation_reason=None,
        )
        
        assert result.intent == "Test intent"
        assert result.intent_class == IntentClass.ECONOMIC_FUNCTION
        assert result.can_handle is True
        assert result.evaluator_name == "EconomicFunctionEvaluator"
        assert result.used_generation is False
