"""
Tests for feature detection module.

Coverage:
- DetectedFeature dataclass
- KeywordFeatureDetector
- PatternFeatureDetector
- ContextFeatureDetector
- FeatureRegistry (registration, detection, aggregation)
- create_default_feature_registry
"""

import pytest
from modelado.feature_detection import (
    DetectedFeature,
    FeatureDetector,
    KeywordFeatureDetector,
    PatternFeatureDetector,
    ContextFeatureDetector,
    FeatureRegistry,
    create_default_feature_registry,
)


class TestDetectedFeature:
    """Tests for DetectedFeature dataclass."""
    
    def test_feature_creation(self):
        """Test creating a detected feature."""
        feature = DetectedFeature(
            name="revenue_analysis",
            confidence=0.8,
            evidence=["revenue", "sales"],
            detector="EconomicKeywords",
            metadata={"keyword_count": 2},
        )
        
        assert feature.name == "revenue_analysis"
        assert feature.confidence == 0.8
        assert feature.evidence == ["revenue", "sales"]
        assert feature.detector == "EconomicKeywords"
        assert feature.metadata["keyword_count"] == 2


class TestKeywordFeatureDetector:
    """Tests for KeywordFeatureDetector."""
    
    @pytest.fixture
    def detector(self):
        """Create keyword detector."""
        feature_keywords = {
            "revenue_analysis": ["revenue", "sales", "income"],
            "cost_analysis": ["cost", "expense", "opex"],
        }
        return KeywordFeatureDetector(
            name="TestKeywords",
            feature_keywords=feature_keywords,
        )
    
    def test_detect_single_feature(self, detector):
        """Test detecting a single feature."""
        intent = "Analyze revenue trends over the past year"
        features = detector.detect(intent)
        
        assert len(features) == 1
        assert features[0].name == "revenue_analysis"
        assert features[0].confidence > 0.0
        assert "revenue" in features[0].evidence
        assert features[0].detector == "TestKeywords"
    
    def test_detect_multiple_features(self, detector):
        """Test detecting multiple features."""
        intent = "Compare revenue growth with cost reduction"
        features = detector.detect(intent)
        
        assert len(features) == 2
        feature_names = {f.name for f in features}
        assert "revenue_analysis" in feature_names
        assert "cost_analysis" in feature_names
    
    def test_detect_multiple_keywords_same_feature(self, detector):
        """Test confidence increases with multiple keyword matches."""
        intent = "Analyze revenue, sales, and income trends"
        features = detector.detect(intent)
        
        assert len(features) == 1
        assert features[0].name == "revenue_analysis"
        assert len(features[0].evidence) == 3
        assert features[0].confidence == 1.0  # All 3 keywords matched
    
    def test_no_features_detected(self, detector):
        """Test when no features match."""
        intent = "Something completely unrelated"
        features = detector.detect(intent)
        
        assert len(features) == 0
    
    def test_case_insensitive_default(self, detector):
        """Test case-insensitive matching by default."""
        intent = "REVENUE and COST analysis"
        features = detector.detect(intent)
        
        assert len(features) == 2
    
    def test_case_sensitive_mode(self):
        """Test case-sensitive matching."""
        detector = KeywordFeatureDetector(
            name="CaseSensitive",
            feature_keywords={"revenue_analysis": ["Revenue"]},
            case_sensitive=True,
        )
        
        # Should match
        features1 = detector.detect("Revenue analysis")
        assert len(features1) == 1
        
        # Should not match
        features2 = detector.detect("revenue analysis")
        assert len(features2) == 0
    
    def test_get_capabilities(self, detector):
        """Test getting detector capabilities."""
        caps = detector.get_capabilities()
        
        assert caps["name"] == "TestKeywords"
        assert caps["type"] == "KeywordFeatureDetector"
        assert "revenue_analysis" in caps["features"]
        assert "cost_analysis" in caps["features"]
        assert caps["case_sensitive"] is False


class TestPatternFeatureDetector:
    """Tests for PatternFeatureDetector."""
    
    @pytest.fixture
    def detector(self):
        """Create pattern detector."""
        feature_patterns = {
            "correlation": [r"correlat\w*", r"relationship between"],
            "comparison": [r"compar\w*", r"versus", r"vs\.?"],
        }
        return PatternFeatureDetector(
            name="TestPatterns",
            feature_patterns=feature_patterns,
        )
    
    def test_detect_single_pattern(self, detector):
        """Test detecting a single pattern."""
        intent = "What is the correlation between revenue and costs?"
        features = detector.detect(intent)
        
        assert len(features) == 1
        assert features[0].name == "correlation"
        assert "correlation" in features[0].evidence[0].lower()
    
    def test_detect_multiple_patterns(self, detector):
        """Test detecting multiple patterns."""
        intent = "Compare the correlation between A and B versus C"
        features = detector.detect(intent)
        
        assert len(features) == 2
        feature_names = {f.name for f in features}
        assert "correlation" in feature_names
        assert "comparison" in feature_names
    
    def test_pattern_variations(self, detector):
        """Test pattern matches variations (correlate, correlation, etc.)."""
        intent = "Correlate these variables"
        features = detector.detect(intent)
        
        assert len(features) == 1
        assert features[0].name == "correlation"
    
    def test_multiple_matches_same_pattern(self, detector):
        """Test multiple matches of the same pattern."""
        intent = "Compare A versus B and compare C vs D"
        features = detector.detect(intent)
        
        assert len(features) == 1
        assert features[0].name == "comparison"
        assert len(features[0].evidence) >= 2
    
    def test_no_patterns_detected(self, detector):
        """Test when no patterns match."""
        intent = "Something unrelated"
        features = detector.detect(intent)
        
        assert len(features) == 0
    
    def test_get_capabilities(self, detector):
        """Test getting detector capabilities."""
        caps = detector.get_capabilities()
        
        assert caps["name"] == "TestPatterns"
        assert caps["type"] == "PatternFeatureDetector"
        assert "correlation" in caps["features"]
        assert "comparison" in caps["features"]


class TestContextFeatureDetector:
    """Tests for ContextFeatureDetector."""
    
    @pytest.fixture
    def detector(self):
        """Create context detector."""
        context_mappings = {
            "artifact_type": {
                "spreadsheet": "spreadsheet_operation",
                "document": "document_operation",
            },
            "project_phase": {
                "planning": "planning_phase",
                "execution": "execution_phase",
            },
        }
        return ContextFeatureDetector(
            name="TestContext",
            context_mappings=context_mappings,
        )
    
    def test_detect_from_context(self, detector):
        """Test detecting features from context."""
        intent = "Do something"
        context = {"artifact_type": "spreadsheet"}
        features = detector.detect(intent, context)
        
        assert len(features) == 1
        assert features[0].name == "spreadsheet_operation"
        assert features[0].confidence == 1.0
        assert "artifact_type=spreadsheet" in features[0].evidence
    
    def test_detect_multiple_context_features(self, detector):
        """Test detecting from multiple context keys."""
        intent = "Do something"
        context = {
            "artifact_type": "document",
            "project_phase": "planning",
        }
        features = detector.detect(intent, context)
        
        assert len(features) == 2
        feature_names = {f.name for f in features}
        assert "document_operation" in feature_names
        assert "planning_phase" in feature_names
    
    def test_no_context_provided(self, detector):
        """Test when no context is provided."""
        intent = "Do something"
        features = detector.detect(intent)
        
        assert len(features) == 0
    
    def test_context_key_not_mapped(self, detector):
        """Test when context key is not in mappings."""
        intent = "Do something"
        context = {"unknown_key": "some_value"}
        features = detector.detect(intent, context)
        
        assert len(features) == 0
    
    def test_context_value_not_mapped(self, detector):
        """Test when context value doesn't match any mapping."""
        intent = "Do something"
        context = {"artifact_type": "unknown_type"}
        features = detector.detect(intent, context)
        
        assert len(features) == 0
    
    def test_substring_matching(self, detector):
        """Test substring matching in context values."""
        intent = "Do something"
        context = {"artifact_type": "excel_spreadsheet_v2"}
        features = detector.detect(intent, context)
        
        # Should match "spreadsheet" substring
        assert len(features) == 1
        assert features[0].name == "spreadsheet_operation"
    
    def test_get_capabilities(self, detector):
        """Test getting detector capabilities."""
        caps = detector.get_capabilities()
        
        assert caps["name"] == "TestContext"
        assert caps["type"] == "ContextFeatureDetector"
        assert "artifact_type" in caps["context_keys"]
        assert "project_phase" in caps["context_keys"]


class TestFeatureRegistry:
    """Tests for FeatureRegistry."""
    
    @pytest.fixture
    def registry(self):
        """Create empty registry."""
        return FeatureRegistry()
    
    @pytest.fixture
    def keyword_detector(self):
        """Create keyword detector."""
        return KeywordFeatureDetector(
            name="Keywords",
            feature_keywords={"revenue": ["revenue", "sales"]},
        )
    
    @pytest.fixture
    def pattern_detector(self):
        """Create pattern detector."""
        return PatternFeatureDetector(
            name="Patterns",
            feature_patterns={"correlation": [r"correlat\w*"]},
        )
    
    def test_register_detector(self, registry, keyword_detector):
        """Test registering a detector."""
        registry.register(keyword_detector)
        
        assert len(registry.detectors) == 1
        assert registry.detectors[0].name == "Keywords"
    
    def test_register_multiple_detectors(self, registry, keyword_detector, pattern_detector):
        """Test registering multiple detectors."""
        registry.register(keyword_detector)
        registry.register(pattern_detector)
        
        assert len(registry.detectors) == 2
    
    def test_detect_all_single_detector(self, registry, keyword_detector):
        """Test detecting with a single detector."""
        registry.register(keyword_detector)
        
        features = registry.detect_all("Analyze revenue trends")
        
        assert len(features) == 1
        assert "revenue" in features
        assert features["revenue"].detector == "Keywords"
    
    def test_detect_all_multiple_detectors(self, registry, keyword_detector, pattern_detector):
        """Test detecting with multiple detectors."""
        registry.register(keyword_detector)
        registry.register(pattern_detector)
        
        features = registry.detect_all("Correlate revenue with costs")
        
        assert len(features) == 2
        assert "revenue" in features
        assert "correlation" in features
    
    def test_detect_all_keeps_highest_confidence(self, registry):
        """Test that registry keeps feature with highest confidence."""
        # Create two detectors that find same feature with different confidence
        detector1 = KeywordFeatureDetector(
            name="Detector1",
            feature_keywords={"revenue": ["revenue"]},  # Will match 1/1 = 1.0
        )
        detector2 = KeywordFeatureDetector(
            name="Detector2",
            feature_keywords={"revenue": ["revenue", "sales", "income"]},  # Will match 1/3 = 0.33
        )
        
        registry.register(detector1)
        registry.register(detector2)
        
        features = registry.detect_all("revenue analysis")
        
        assert len(features) == 1
        assert features["revenue"].confidence == 1.0
        assert features["revenue"].detector == "Detector1"
    
    def test_detect_all_no_features(self, registry, keyword_detector):
        """Test when no features are detected."""
        registry.register(keyword_detector)
        
        features = registry.detect_all("Something unrelated")
        
        assert len(features) == 0
    
    def test_detect_all_with_context(self, registry):
        """Test detecting with context."""
        context_detector = ContextFeatureDetector(
            name="Context",
            context_mappings={"artifact_type": {"spreadsheet": "spreadsheet_op"}},
        )
        registry.register(context_detector)
        
        features = registry.detect_all(
            "Do something",
            context={"artifact_type": "spreadsheet"},
        )
        
        assert len(features) == 1
        assert "spreadsheet_op" in features
    
    def test_detect_all_handles_detector_errors(self, registry):
        """Test that registry handles detector failures gracefully."""
        # Create a detector that will raise an error
        class FailingDetector(FeatureDetector):
            def detect(self, intent, context=None):
                raise ValueError("Detector failed!")
        
        failing_detector = FailingDetector("Failing")
        keyword_detector = KeywordFeatureDetector(
            name="Working",
            feature_keywords={"revenue": ["revenue"]},
        )
        
        registry.register(failing_detector)
        registry.register(keyword_detector)
        
        # Should still get features from working detector
        features = registry.detect_all("revenue analysis")
        
        assert len(features) == 1
        assert "revenue" in features
    
    def test_get_capabilities(self, registry, keyword_detector, pattern_detector):
        """Test getting registry capabilities."""
        registry.register(keyword_detector)
        registry.register(pattern_detector)
        
        caps = registry.get_capabilities()
        
        assert caps["detector_count"] == 2
        assert len(caps["detectors"]) == 2
        assert caps["detectors"][0]["name"] == "Keywords"
        assert caps["detectors"][1]["name"] == "Patterns"


class TestDefaultFeatureRegistry:
    """Tests for create_default_feature_registry."""
    
    def test_creates_registry_with_detectors(self):
        """Test that default registry has detectors."""
        registry = create_default_feature_registry()
        
        assert len(registry.detectors) > 0
    
    def test_detects_economic_features(self):
        """Test detecting economic features."""
        registry = create_default_feature_registry()
        
        features = registry.detect_all("Analyze revenue and cost trends")
        
        assert "revenue_analysis" in features
        assert "cost_analysis" in features
    
    def test_detects_story_features(self):
        """Test detecting story features."""
        registry = create_default_feature_registry()
        
        features = registry.detect_all("Create a narrative for investor slides")
        
        assert "narrative" in features
        assert "slides" in features
    
    def test_detects_analytical_patterns(self):
        """Test detecting analytical patterns."""
        registry = create_default_feature_registry()
        
        features = registry.detect_all("Correlate revenue with market trends")
        
        assert "correlation" in features
    
    def test_detects_context_features(self):
        """Test detecting context-based features."""
        registry = create_default_feature_registry()
        
        features = registry.detect_all(
            "Do something",
            context={"artifact_type": "spreadsheet"},
        )
        
        assert "spreadsheet_operation" in features
    
    def test_combined_detection(self):
        """Test detecting features from multiple detector types."""
        registry = create_default_feature_registry()
        
        features = registry.detect_all(
            "Compare revenue trends in this spreadsheet",
            context={"artifact_type": "excel"},
        )
        
        # Should detect: revenue_analysis, comparison, spreadsheet_operation, trend
        assert "revenue_analysis" in features
        assert "comparison" in features
        assert "spreadsheet_operation" in features
        assert "trend" in features
