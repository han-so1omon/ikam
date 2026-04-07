"""
Feature Detection for Semantic Evaluation.

This module provides centralized feature detection logic for analyzing intents
and contexts. Features are semantic signals that help evaluators determine
whether they can handle a given intent.

Architecture:
    - FeatureDetector: Base class for all feature detectors
    - FeatureRegistry: Central registry for feature detectors
    - DetectedFeature: Result of feature detection with confidence
    
Feature Types:
    - Keyword-based: Detect presence of specific terms
    - Pattern-based: Regex or structural patterns
    - Semantic-based: Embedding similarity to feature exemplars
    - Context-based: Features derived from context metadata
    
Design Principles:
    - Composable: Multiple detectors can run in parallel
    - Confident: Each feature has a confidence score (0.0-1.0)
    - Extensible: Easy to add new feature types
    - Observable: Log all detected features for debugging
"""

from abc import ABC, abstractmethod
from dataclasses import dataclass
from datetime import datetime
from typing import Any, Dict, List, Optional, Set
import logging
import re

logger = logging.getLogger(__name__)


@dataclass
class DetectedFeature:
    """
    Result of feature detection.
    
    Attributes:
        name: Feature name (e.g., 'revenue_analysis', 'cost_driver')
        confidence: Confidence score 0.0-1.0
        evidence: Evidence supporting detection (keywords, patterns, etc.)
        detector: Name of detector that found this feature
        metadata: Additional detector-specific metadata
    """
    name: str
    confidence: float
    evidence: List[str]
    detector: str
    metadata: Dict[str, Any]


class FeatureDetector(ABC):
    """
    Base class for feature detectors.
    
    Each detector identifies specific semantic features in intent/context.
    """
    
    def __init__(self, name: str):
        """
        Initialize detector.
        
        Args:
            name: Detector name for logging/debugging
        """
        self.name = name
    
    @abstractmethod
    def detect(self, intent: str, context: Optional[Dict[str, Any]] = None) -> List[DetectedFeature]:
        """
        Detect features in intent and context.
        
        Args:
            intent: User intent text
            context: Optional context metadata
            
        Returns:
            List of detected features with confidence scores
        """
        pass
    
    def get_capabilities(self) -> Dict[str, Any]:
        """
        Get detector capabilities.
        
        Returns:
            Dictionary describing detector capabilities
        """
        return {
            "name": self.name,
            "type": self.__class__.__name__,
        }


class KeywordFeatureDetector(FeatureDetector):
    """
    Detect features based on keyword presence.
    
    Simple but effective for domain-specific terms.
    """
    
    def __init__(
        self,
        name: str,
        feature_keywords: Dict[str, List[str]],
        case_sensitive: bool = False,
    ):
        """
        Initialize keyword detector.
        
        Args:
            name: Detector name
            feature_keywords: Map of feature_name -> list of keywords
            case_sensitive: Whether to match case-sensitively
        """
        super().__init__(name)
        self.feature_keywords = feature_keywords
        self.case_sensitive = case_sensitive
    
    def detect(self, intent: str, context: Optional[Dict[str, Any]] = None) -> List[DetectedFeature]:
        """Detect features by keyword matching."""
        features = []
        search_text = intent if self.case_sensitive else intent.lower()
        
        for feature_name, keywords in self.feature_keywords.items():
            matches = []
            for keyword in keywords:
                search_keyword = keyword if self.case_sensitive else keyword.lower()
                if search_keyword in search_text:
                    matches.append(keyword)
            
            if matches:
                # Confidence based on number of keyword matches
                confidence = min(1.0, len(matches) / len(keywords))
                features.append(DetectedFeature(
                    name=feature_name,
                    confidence=confidence,
                    evidence=matches,
                    detector=self.name,
                    metadata={"keyword_count": len(matches)},
                ))
        
        return features
    
    def get_capabilities(self) -> Dict[str, Any]:
        """Get detector capabilities."""
        base = super().get_capabilities()
        base.update({
            "features": list(self.feature_keywords.keys()),
            "case_sensitive": self.case_sensitive,
        })
        return base


class PatternFeatureDetector(FeatureDetector):
    """
    Detect features based on regex patterns.
    
    Useful for structural patterns like "increase X by Y%" or "correlate A with B".
    """
    
    def __init__(
        self,
        name: str,
        feature_patterns: Dict[str, List[str]],
        flags: int = re.IGNORECASE,
    ):
        """
        Initialize pattern detector.
        
        Args:
            name: Detector name
            feature_patterns: Map of feature_name -> list of regex patterns
            flags: Regex flags (default: case-insensitive)
        """
        super().__init__(name)
        self.feature_patterns = {
            feature: [re.compile(pattern, flags) for pattern in patterns]
            for feature, patterns in feature_patterns.items()
        }
        self.flags = flags
    
    def detect(self, intent: str, context: Optional[Dict[str, Any]] = None) -> List[DetectedFeature]:
        """Detect features by pattern matching."""
        features = []
        
        for feature_name, patterns in self.feature_patterns.items():
            matches = []
            for pattern in patterns:
                for match in pattern.finditer(intent):
                    matches.append(match.group(0))
            
            if matches:
                # Confidence based on pattern match count
                confidence = min(1.0, len(matches) / len(patterns))
                features.append(DetectedFeature(
                    name=feature_name,
                    confidence=confidence,
                    evidence=matches,
                    detector=self.name,
                    metadata={"match_count": len(matches)},
                ))
        
        return features
    
    def get_capabilities(self) -> Dict[str, Any]:
        """Get detector capabilities."""
        base = super().get_capabilities()
        base.update({
            "features": list(self.feature_patterns.keys()),
            "regex_flags": self.flags,
        })
        return base


class ContextFeatureDetector(FeatureDetector):
    """
    Detect features based on context metadata.
    
    Examines context fields like artifact type, project phase, user role, etc.
    """
    
    def __init__(
        self,
        name: str,
        context_mappings: Dict[str, Dict[str, str]],
    ):
        """
        Initialize context detector.
        
        Args:
            name: Detector name
            context_mappings: Map of context_key -> {value: feature_name}
                Example: {"artifact_type": {"spreadsheet": "spreadsheet_operation"}}
        """
        super().__init__(name)
        self.context_mappings = context_mappings
    
    def detect(self, intent: str, context: Optional[Dict[str, Any]] = None) -> List[DetectedFeature]:
        """Detect features from context metadata."""
        features = []
        
        if not context:
            return features
        
        for context_key, value_mapping in self.context_mappings.items():
            if context_key in context:
                context_value = str(context[context_key])
                
                for value_pattern, feature_name in value_mapping.items():
                    # Support exact match or substring match
                    if value_pattern.lower() in context_value.lower():
                        features.append(DetectedFeature(
                            name=feature_name,
                            confidence=1.0,  # High confidence for context matches
                            evidence=[f"{context_key}={context_value}"],
                            detector=self.name,
                            metadata={"context_key": context_key},
                        ))
        
        return features
    
    def get_capabilities(self) -> Dict[str, Any]:
        """Get detector capabilities."""
        base = super().get_capabilities()
        base.update({
            "context_keys": list(self.context_mappings.keys()),
        })
        return base


class FeatureRegistry:
    """
    Central registry for feature detectors.
    
    Manages multiple detectors and provides unified feature detection interface.
    """
    
    def __init__(self):
        """Initialize empty registry."""
        self.detectors: List[FeatureDetector] = []
    
    def register(self, detector: FeatureDetector) -> None:
        """
        Register a feature detector.
        
        Args:
            detector: Detector to register
        """
        self.detectors.append(detector)
        logger.info(f"Registered feature detector: {detector.name}")
    
    def detect_all(
        self,
        intent: str,
        context: Optional[Dict[str, Any]] = None,
    ) -> Dict[str, DetectedFeature]:
        """
        Run all detectors and aggregate results.
        
        If multiple detectors find the same feature, keep the highest confidence.
        
        Args:
            intent: User intent text
            context: Optional context metadata
            
        Returns:
            Dictionary mapping feature_name -> DetectedFeature (best match)
        """
        all_features: Dict[str, DetectedFeature] = {}
        
        for detector in self.detectors:
            try:
                features = detector.detect(intent, context)
                
                for feature in features:
                    # Keep feature with highest confidence
                    if feature.name not in all_features or feature.confidence > all_features[feature.name].confidence:
                        all_features[feature.name] = feature
                        logger.debug(
                            f"Feature detected: {feature.name} (confidence={feature.confidence:.2f}, "
                            f"detector={feature.detector}, evidence={feature.evidence})"
                        )
            
            except Exception as e:
                logger.error(f"Feature detector {detector.name} failed: {e}", exc_info=True)
        
        return all_features
    
    def get_capabilities(self) -> Dict[str, Any]:
        """
        Get all detector capabilities.
        
        Returns:
            Dictionary with detector capabilities and statistics
        """
        return {
            "detector_count": len(self.detectors),
            "detectors": [detector.get_capabilities() for detector in self.detectors],
        }


def create_default_feature_registry() -> FeatureRegistry:
    """
    Create a default feature registry with common detectors.
    
    This includes detectors for:
    - Economic operations (revenue, costs, margins, etc.)
    - Story operations (narrative, slides, themes, etc.)
    - Artifact types (spreadsheet, document, presentation, etc.)
    - Analytical operations (sensitivity, scenario, correlation, etc.)
    
    Returns:
        Configured FeatureRegistry
    """
    registry = FeatureRegistry()
    
    # Economic keyword detector
    economic_keywords = {
        "revenue_analysis": ["revenue", "sales", "income", "earnings", "top line"],
        "cost_analysis": ["cost", "expense", "opex", "capex", "burn rate"],
        "margin_analysis": ["margin", "gross margin", "contribution margin", "ebitda"],
        "unit_economics": ["unit economics", "cac", "ltv", "customer acquisition"],
        "forecast": ["forecast", "projection", "predict", "estimate future"],
        "scenario": ["scenario", "what if", "sensitivity", "monte carlo"],
    }
    registry.register(KeywordFeatureDetector(
        name="EconomicKeywords",
        feature_keywords=economic_keywords,
    ))
    
    # Story keyword detector
    story_keywords = {
        "narrative": ["narrative", "story", "tell", "explain", "communicate"],
        "slides": ["slide", "deck", "presentation", "powerpoint"],
        "theme": ["theme", "message", "angle", "positioning"],
        "audience": ["audience", "investor", "board", "stakeholder"],
    }
    registry.register(KeywordFeatureDetector(
        name="StoryKeywords",
        feature_keywords=story_keywords,
    ))
    
    # Analytical pattern detector
    analytical_patterns = {
        "correlation": [r"correlat\w*", r"relationship between", r"impact of \w+ on"],
        "comparison": [r"compar\w*", r"versus", r"vs\.?", r"difference between"],
        "trend": [r"trend\w*", r"over time", r"historical", r"growth rate"],
        "aggregation": [r"total", r"sum", r"average", r"aggregate"],
    }
    registry.register(PatternFeatureDetector(
        name="AnalyticalPatterns",
        feature_patterns=analytical_patterns,
    ))
    
    # Context-based detector
    context_mappings = {
        "artifact_type": {
            "spreadsheet": "spreadsheet_operation",
            "sheet": "spreadsheet_operation",
            "excel": "spreadsheet_operation",
            "document": "document_operation",
            "slides": "slides_operation",
            "presentation": "slides_operation",
        },
        "project_phase": {
            "planning": "planning_phase",
            "execution": "execution_phase",
            "review": "review_phase",
        },
    }
    registry.register(ContextFeatureDetector(
        name="ContextMetadata",
        context_mappings=context_mappings,
    ))
    
    logger.info(f"Created default feature registry with {len(registry.detectors)} detectors")
    return registry
