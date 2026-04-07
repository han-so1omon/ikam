"""Function generation strategies.

This package implements multiple approaches to generating executable functions from
user intent + semantic context:

1. **LLM-based generation** (`llm_generator.py`):
   - Uses GPT-4o-mini with deterministic seed for code generation
   - Tracks token usage and cost
   - Caches by (intent + semantic_features + model_version + seed)

2. **Composable building blocks** (`composable_builder.py`):
   - Composes functions from pre-validated atomic operations
   - Fully deterministic (no LLM involved)
   - Fast and low-cost

3. **Template injection** (`template_injector.py`):
   - Fills templates with extracted parameters from semantic analysis
   - Balanced: deterministic structure + flexible parameter slots
   - Current template approach (enhanced with parameter extraction)

4. **Strategy selector** (`strategy_selector.py`):
   - Chooses generation strategy based on intent complexity, semantic features, cost/latency constraints
   - Provides observability for strategy selection decisions

All generators follow the `FunctionGenerator` protocol and return `GeneratedOperation`.

Mathematical guarantees:
- Determinism: Same (command + strategy + seed) → same generated function
- Cost control: Token tracking + caching prevent redundant LLM calls
- Provenance: Full generation metadata (strategy, model, seed, tokens, cost) recorded
"""

from modelado.core.function_generators.base import FunctionGenerator, GenerationContext
from modelado.core.function_generators.llm_generator import LLMFunctionGenerator
from modelado.core.function_generators.composable_builder import ComposableFunctionBuilder
from modelado.core.function_generators.template_injector import TemplateInjector
from modelado.core.function_generators.strategy_selector import StrategySelector

__all__ = [
    "FunctionGenerator",
    "GenerationContext",
    "LLMFunctionGenerator",
    "ComposableFunctionBuilder",
    "TemplateInjector",
    "StrategySelector",
]
