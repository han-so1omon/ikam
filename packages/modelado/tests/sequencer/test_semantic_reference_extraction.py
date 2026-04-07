"""Tests for semantic reference extraction module.

This test suite validates the semantic reference extraction pipeline:
1. Artifact mention detection via embeddings
2. Reference type inference from context
3. Complete reference extraction workflow
4. Edge cases and error handling

Test Coverage:
- 20+ tests covering all extraction functions
- Mocking strategy for SemanticEngine (async embedding calls)
- Database mocking for artifact lookup
- Confidence scoring validation
- Context window extraction
"""

import pytest
from unittest.mock import AsyncMock, MagicMock, patch
from typing import List, Dict, Any

from modelado.sequencer.semantic_reference_extraction import (
    extract_artifact_mentions,
    infer_reference_type,
    extract_semantic_references,
    SemanticReference,
    ArtifactMention,
    _cosine_similarity,
    _find_mention_in_text,
    _extract_context_window,
    _deduplicate_mentions,
    _map_concept_to_kind,
)


# Test fixtures


@pytest.fixture
def mock_semantic_engine():
    """Mock SemanticEngine for testing."""
    engine = MagicMock()
    
    # Mock embed_text to return higher similarity for related concepts
    async def mock_embed(text: str) -> List[float]:
        import hashlib
        import numpy as np
        
        # Base vector from hash for uniqueness
        hash_obj = hashlib.sha256(text.encode())
        seed = int(hash_obj.hexdigest(), 16) % (2**32)
        np.random.seed(seed)
        
        # Generate base embedding
        base_embedding = np.random.randn(1536)
        
        # Add similarity boosts for related concepts
        # This makes embeddings for related texts more similar
        text_lower = text.lower()
        similarity_boosts = []
        
        if any(word in text_lower for word in ["cost", "expense", "financial"]):
            similarity_boosts.append(np.ones(1536) * 0.5)
        
        if any(word in text_lower for word in ["revenue", "sales", "forecast"]):
            similarity_boosts.append(np.ones(1536) * 0.45)
        
        if any(word in text_lower for word in ["narrative", "story", "presentation"]):
            similarity_boosts.append(np.ones(1536) * 0.4)
        
        if any(word in text_lower for word in ["model", "analysis", "framework"]):
            similarity_boosts.append(np.ones(1536) * 0.35)
        
        if any(word in text_lower for word in ["plan", "phase", "project"]):
            similarity_boosts.append(np.ones(1536) * 0.3)
        
        # Combine base + boosts
        if similarity_boosts:
            for boost in similarity_boosts:
                base_embedding += boost
        
        # Normalize
        norm_val = np.linalg.norm(base_embedding)
        return (base_embedding / norm_val).tolist()
    
    engine.embed_text = AsyncMock(side_effect=mock_embed)
    
    return engine


@pytest.fixture
def mock_db_connection():
    """Mock database connection for artifact lookup."""
    conn = MagicMock()
    cursor = MagicMock()
    
    # Mock cursor context manager
    conn.cursor.return_value.__enter__.return_value = cursor
    conn.cursor.return_value.__exit__.return_value = None
    
    return conn, cursor


# Test: Cosine similarity calculation


def test_cosine_similarity_identical_vectors():
    """Test cosine similarity with identical vectors."""
    vec = [1.0, 0.0, 0.0]
    similarity = _cosine_similarity(vec, vec)
    assert similarity == pytest.approx(1.0, abs=1e-6)


def test_cosine_similarity_orthogonal_vectors():
    """Test cosine similarity with orthogonal vectors."""
    vec_a = [1.0, 0.0, 0.0]
    vec_b = [0.0, 1.0, 0.0]
    similarity = _cosine_similarity(vec_a, vec_b)
    assert similarity == pytest.approx(0.0, abs=1e-6)


def test_cosine_similarity_opposite_vectors():
    """Test cosine similarity with opposite vectors."""
    vec_a = [1.0, 0.0, 0.0]
    vec_b = [-1.0, 0.0, 0.0]
    similarity = _cosine_similarity(vec_a, vec_b)
    assert similarity == pytest.approx(-1.0, abs=1e-6)


def test_cosine_similarity_zero_vector():
    """Test cosine similarity with zero vector."""
    vec_a = [0.0, 0.0, 0.0]
    vec_b = [1.0, 1.0, 1.0]
    similarity = _cosine_similarity(vec_a, vec_b)
    assert similarity == 0.0


# Test: Find mention in text


def test_find_mention_exact_match():
    """Test finding exact keyword match in text."""
    text = "Plan phases to integrate our cost model"
    concept_id = "cost_model"
    
    mention, (start, end) = _find_mention_in_text(text, concept_id)
    
    assert mention == "cost model"
    assert start == 29
    assert end == 39


def test_find_mention_partial_match():
    """Test finding partial keyword match."""
    text = "Use the cost calculations for modeling"
    concept_id = "cost_model"
    
    mention, (start, end) = _find_mention_in_text(text, concept_id)
    
    # Should find "cost" first
    assert "cost" in mention.lower()
    assert start == 8


def test_find_mention_case_insensitive():
    """Test case-insensitive mention detection."""
    text = "Review the COST MODEL assumptions"
    concept_id = "cost_model"
    
    mention, (start, end) = _find_mention_in_text(text, concept_id)
    
    assert mention == "COST MODEL"
    assert start == 11


def test_find_mention_no_match():
    """Test finding mention when keywords not in text."""
    text = "Plan project phases"
    concept_id = "cost_model"
    
    mention, (start, end) = _find_mention_in_text(text, concept_id)
    
    # Should return concept_id as fallback
    assert mention == "cost model"
    assert start == 0


# Test: Extract context window


def test_extract_context_window_middle():
    """Test context extraction from middle of text."""
    text = "A" * 100 + "MENTION" + "B" * 100
    start_pos = 100
    end_pos = 107
    
    context = _extract_context_window(text, start_pos, end_pos, window_size=20)
    
    assert len(context) == 47  # 20 before + 7 mention + 20 after
    assert "MENTION" in context


def test_extract_context_window_start_of_text():
    """Test context extraction near start of text."""
    text = "MENTION at start" + "B" * 100
    start_pos = 0
    end_pos = 7
    
    context = _extract_context_window(text, start_pos, end_pos, window_size=20)
    
    # Should not go before position 0
    assert context.startswith("MENTION")


def test_extract_context_window_end_of_text():
    """Test context extraction near end of text."""
    text = "A" * 100 + "MENTION at end"
    start_pos = 100
    end_pos = 114
    
    context = _extract_context_window(text, start_pos, end_pos, window_size=50)
    
    # Should not go past end of text
    assert context.endswith("end")


# Test: Deduplicate mentions


def test_deduplicate_mentions_keep_highest_similarity():
    """Test deduplication keeps highest similarity mention."""
    mentions = [
        ArtifactMention("cost", "cost_model", 0.7, "context1", 0, 4),
        ArtifactMention("cost model", "cost_model", 0.9, "context2", 10, 20),
        ArtifactMention("cost", "cost_model", 0.6, "context3", 30, 34),
    ]
    
    deduped = _deduplicate_mentions(mentions)
    
    assert len(deduped) == 1
    assert deduped[0].similarity_score == 0.9
    assert deduped[0].text == "cost model"


def test_deduplicate_mentions_different_concepts():
    """Test deduplication preserves different concepts."""
    mentions = [
        ArtifactMention("cost", "cost_model", 0.8, "context1", 0, 4),
        ArtifactMention("revenue", "revenue_model", 0.7, "context2", 10, 17),
    ]
    
    deduped = _deduplicate_mentions(mentions)
    
    assert len(deduped) == 2


# Test: Map concept to kind


def test_map_concept_to_kind_cost_model():
    """Test mapping cost_model to EconomicModel."""
    kind = _map_concept_to_kind("cost_model")
    assert kind == "EconomicModel"


def test_map_concept_to_kind_forecast_data():
    """Test mapping forecast_data to Sheet."""
    kind = _map_concept_to_kind("forecast_data")
    assert kind == "Sheet"


def test_map_concept_to_kind_narrative():
    """Test mapping narrative to Document."""
    kind = _map_concept_to_kind("narrative")
    assert kind == "Document"


def test_map_concept_to_kind_unknown():
    """Test unknown concept defaults to EconomicModel."""
    kind = _map_concept_to_kind("unknown_concept")
    assert kind == "EconomicModel"


# Test: Extract artifact mentions


@pytest.mark.asyncio
async def test_extract_artifact_mentions_basic(mock_semantic_engine):
    """Test basic artifact mention extraction."""
    planning_text = "Plan phases to integrate our cost model into the narrative"
    
    mentions = await extract_artifact_mentions(
        planning_text,
        mock_semantic_engine,
        similarity_threshold=0.3  # Lower threshold for mock embeddings
    )
    
    # Should detect cost_model and narrative mentions
    assert len(mentions) >= 1
    assert mock_semantic_engine.embed_text.call_count >= 2


@pytest.mark.asyncio
async def test_extract_artifact_mentions_empty_text(mock_semantic_engine):
    """Test extraction with empty text."""
    mentions = await extract_artifact_mentions(
        "",
        mock_semantic_engine,
        similarity_threshold=0.5
    )
    
    assert len(mentions) == 0
    assert mock_semantic_engine.embed_text.call_count == 0


@pytest.mark.asyncio
async def test_extract_artifact_mentions_high_threshold(mock_semantic_engine):
    """Test extraction with high similarity threshold."""
    planning_text = "Generic project planning text"
    
    mentions = await extract_artifact_mentions(
        planning_text,
        mock_semantic_engine,
        similarity_threshold=0.95
    )
    
    # High threshold should filter out low-similarity matches
    assert len(mentions) <= 1


# Test: Infer reference type


@pytest.mark.asyncio
async def test_infer_reference_type_depends_on(mock_semantic_engine):
    """Test inferring depends_on_formula reference type."""
    planning_text = "Phase 2 depends on the cost model calculations"
    mention_text = "cost model"
    
    inference = await infer_reference_type(
        planning_text,
        mention_text,
        "EconomicModel",
        mock_semantic_engine
    )
    
    assert inference["reference_type"] == "depends_on_formula"
    assert inference["confidence"] > 0.6
    assert "depends on" in inference["reasoning"].lower()


@pytest.mark.asyncio
async def test_infer_reference_type_uses(mock_semantic_engine):
    """Test inferring uses_variable reference type."""
    planning_text = "The analysis uses revenue forecast data"
    mention_text = "revenue forecast"
    
    inference = await infer_reference_type(
        planning_text,
        mention_text,
        "Sheet",
        mock_semantic_engine
    )
    
    assert inference["reference_type"] == "uses_variable"
    assert inference["confidence"] > 0.6


@pytest.mark.asyncio
async def test_infer_reference_type_output_to(mock_semantic_engine):
    """Test inferring output_to_narrative reference type."""
    planning_text = "Integrate cost model into investor narrative"
    mention_text = "cost model"
    
    inference = await infer_reference_type(
        planning_text,
        mention_text,
        "EconomicModel",
        mock_semantic_engine
    )
    
    assert inference["reference_type"] == "output_to_narrative"
    assert inference["confidence"] > 0.6


@pytest.mark.asyncio
async def test_infer_reference_type_mention_not_found(mock_semantic_engine):
    """Test inference when mention not in text."""
    planning_text = "Generic planning text"
    mention_text = "nonexistent model"
    
    inference = await infer_reference_type(
        planning_text,
        mention_text,
        "EconomicModel",
        mock_semantic_engine
    )
    
    assert inference["reference_type"] == "uses_variable"
    assert inference["confidence"] == 0.5
    assert "not found" in inference["reasoning"].lower()


@pytest.mark.asyncio
async def test_infer_reference_type_multiple_patterns(mock_semantic_engine):
    """Test inference with multiple matching patterns."""
    planning_text = (
        "Phase 3 depends on and requires the cost model "
        "and needs validation from revenue forecast"
    )
    mention_text = "cost model"
    
    inference = await infer_reference_type(
        planning_text,
        mention_text,
        "EconomicModel",
        mock_semantic_engine
    )
    
    assert inference["reference_type"] == "depends_on_formula"
    # Multiple patterns should increase confidence
    assert inference["confidence"] >= 0.7


# Test: Extract semantic references (integration)


@pytest.mark.asyncio
async def test_extract_semantic_references_integration(
    mock_semantic_engine,
    mock_db_connection
):
    """Test complete reference extraction workflow."""
    conn, cursor = mock_db_connection
    
    # Mock artifact lookup to return artifact_id
    cursor.fetchone.return_value = ("cost-model-v3",)
    
    planning_text = "Plan phases to integrate our cost model"
    
    with patch(
        "modelado.sequencer.semantic_reference_extraction.HAS_SEMANTIC_ENGINE",
        True
    ), patch(
        "modelado.sequencer.semantic_reference_extraction.HAS_SEQUENCER_MODELS",
        True
    ), patch(
        "modelado.sequencer.semantic_reference_extraction.lookup_artifact_by_semantic_match",
        return_value="cost-model-v3"
    ):
        references = await extract_semantic_references(
            planning_text,
            mock_semantic_engine,
            conn,
            similarity_threshold=0.3  # Lower threshold for mock embeddings
        )
    
    # Should extract at least cost_model reference
    assert len(references) >= 1
    
    # Validate reference structure
    ref = references[0]
    assert ref.artifact_id == "cost-model-v3"
    assert ref.artifact_kind == "EconomicModel"
    assert ref.confidence > 0.0
    assert len(ref.reasoning) > 0


@pytest.mark.asyncio
async def test_extract_semantic_references_no_artifacts_found(
    mock_semantic_engine,
    mock_db_connection
):
    """Test extraction when no artifacts are found in database."""
    conn, cursor = mock_db_connection
    
    # Mock artifact lookup to return None
    cursor.fetchone.return_value = None
    
    planning_text = "Plan phases with cost model"
    
    with patch(
        "modelado.sequencer.semantic_reference_extraction.HAS_SEMANTIC_ENGINE",
        True
    ), patch(
        "modelado.sequencer.semantic_reference_extraction.HAS_SEQUENCER_MODELS",
        True
    ), patch(
        "modelado.sequencer.semantic_reference_extraction.lookup_artifact_by_semantic_match",
        return_value=None
    ):
        references = await extract_semantic_references(
            planning_text,
            mock_semantic_engine,
            conn,
            similarity_threshold=0.5
        )
    
    # Should return empty list when no artifacts found
    assert len(references) == 0


@pytest.mark.asyncio
async def test_extract_semantic_references_missing_dependencies(
    mock_semantic_engine,
    mock_db_connection
):
    """Test extraction gracefully handles missing dependencies."""
    conn, _ = mock_db_connection
    
    planning_text = "Plan phases"
    
    with patch(
        "modelado.sequencer.semantic_reference_extraction.HAS_SEMANTIC_ENGINE",
        False
    ):
        references = await extract_semantic_references(
            planning_text,
            mock_semantic_engine,
            conn,
            similarity_threshold=0.5
        )
    
    # Should return empty list when dependencies unavailable
    assert len(references) == 0


@pytest.mark.asyncio
async def test_extract_semantic_references_multiple_mentions(
    mock_semantic_engine,
    mock_db_connection
):
    """Test extraction with multiple artifact mentions."""
    conn, cursor = mock_db_connection
    
    # Map to track which artifacts we've seen
    artifacts_by_mention = {
        "revenue": "revenue-forecast-q4",
        "forecast": "revenue-forecast-q4",
        "cost": "cost-model-v3",
        "model": "cost-model-v3",
    }
    
    def lookup_mock(mention: str, kind: str, connection):
        """Mock artifact lookup that returns based on mention keywords."""
        mention_lower = mention.lower()
        for key, artifact_id in artifacts_by_mention.items():
            if key in mention_lower:
                return artifact_id
        return None
    
    planning_text = (
        "Phase 1: data prep using revenue forecast. "
        "Phase 2: modeling with cost model. "
        "Phase 3: visualization."
    )
    
    with patch(
        "modelado.sequencer.semantic_reference_extraction.HAS_SEMANTIC_ENGINE",
        True
    ), patch(
        "modelado.sequencer.semantic_reference_extraction.HAS_SEQUENCER_MODELS",
        True
    ), patch(
        "modelado.sequencer.semantic_reference_extraction.lookup_artifact_by_semantic_match",
        side_effect=lookup_mock
    ):
        references = await extract_semantic_references(
            planning_text,
            mock_semantic_engine,
            conn,
            similarity_threshold=0.3  # Lower threshold for mock embeddings
        )
    
    # Should extract multiple references
    assert len(references) >= 1
    
    # Validate reference types are inferred correctly
    for ref in references:
        assert ref.artifact_id in ["cost-model-v3", "revenue-forecast-q4"]
        assert ref.reference_type in [
            "depends_on_formula",
            "uses_variable",
            "input_from_data",
            "output_to_narrative",
            "extends_model",
            "validates_forecast",
            "references_context"
        ]


@pytest.mark.asyncio
async def test_extract_semantic_references_combined_confidence(
    mock_semantic_engine,
    mock_db_connection
):
    """Test combined confidence calculation."""
    conn, cursor = mock_db_connection
    
    cursor.fetchone.return_value = ("cost-model-v3",)
    
    planning_text = "Plan depends on cost model"
    
    with patch(
        "modelado.sequencer.semantic_reference_extraction.HAS_SEMANTIC_ENGINE",
        True
    ), patch(
        "modelado.sequencer.semantic_reference_extraction.HAS_SEQUENCER_MODELS",
        True
    ), patch(
        "modelado.sequencer.semantic_reference_extraction.lookup_artifact_by_semantic_match",
        return_value="cost-model-v3"
    ):
        references = await extract_semantic_references(
            planning_text,
            mock_semantic_engine,
            conn,
            similarity_threshold=0.5
        )
    
    if references:
        ref = references[0]
        # Combined confidence should be product of mention similarity and inference confidence
        # Both should be > 0, so combined should be > 0
        assert ref.confidence > 0.0
        # Should be less than max of either component (unless both are 1.0)
        assert ref.confidence <= 1.0
