"""Test helpers for Phase 9.3 function generators.

Provides fixtures and utilities for testing generator implementations.
"""

from modelado.core.generative_contracts import GenerativeCommand
from modelado.core.function_generators.base import GenerationContext


def create_test_command(
    user_instruction: str,
    operation_type: str = "economic_function",
    context: dict = None,
) -> GenerativeCommand:
    """Create a test GenerativeCommand.
    
    Args:
        user_instruction: User's natural language instruction
        operation_type: Type of operation
        context: Context dict
    
    Returns:
        GenerativeCommand instance
    """
    return GenerativeCommand.create(
        user_instruction=user_instruction,
        operation_type=operation_type,
        context=context or {},
    )


def create_test_context(
    user_instruction: str,
    intent_type: str = "sensitivity_analysis",
    semantic_features: dict = None,
    intent_confidence: float = 0.9,
    seed: int = 42,
) -> GenerationContext:
    """Create a test GenerationContext.
    
    Args:
        user_instruction: User's instruction
        intent_type: Type of intent
        semantic_features: Detected semantic features
        intent_confidence: Confidence score
        seed: Random seed for reproducibility
    
    Returns:
        GenerationContext instance
    """
    command = create_test_command(
        user_instruction=user_instruction,
        operation_type="economic_function",
        context={},
    )
    
    return GenerationContext(
        command=command,
        semantic_features=semantic_features or {},
        intent_type=intent_type,
        intent_confidence=intent_confidence,
        cost_budget_usd=0.01,
        latency_budget_ms=1000,
        seed=seed,
    )
