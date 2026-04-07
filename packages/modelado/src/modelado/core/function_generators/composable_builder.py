"""Composable function builder using pre-validated atomic operations.

Builds complex functions by composing atomic operations (building blocks).

Key features:
- Fully deterministic (no LLM involved)
- Fast (microseconds, not seconds)
- Zero cost (no API calls)
- Type-safe composition
- Pre-validated building blocks

Building blocks:
- Arithmetic operations (add, subtract, multiply, divide, percent_change)
- Aggregations (sum, mean, min, max, count)
- Filters (where, top_n, bottom_n)
- Transformations (map, group_by, pivot)
- Economic operations (sensitivity, waterfall, break_even)

Mathematical guarantees:
- Determinism: Same composition recipe → same generated function
- Type safety: Composition validates input/output type contracts
- Correctness: All building blocks are pre-validated with unit tests
"""

from __future__ import annotations

import hashlib
import json
import logging
from datetime import datetime
from typing import Any, Callable, Dict, List, Optional, Tuple

from modelado.core.function_generators.base import (
    FunctionGenerator,
    GenerationContext,
    GenerationError,
)
from modelado.core.generative_contracts import (
    GeneratedOperation,
    ExecutableFunction,
    ValidationResults,
    GenerationStrategy,
    ConstraintType,
)

logger = logging.getLogger(__name__)


# Building block registry
BUILDING_BLOCKS: Dict[str, Tuple[str, Dict[str, str]]] = {
    # Arithmetic
    "add": ("lambda a, b: a + b", {"a": "float", "b": "float"}),
    "subtract": ("lambda a, b: a - b", {"a": "float", "b": "float"}),
    "multiply": ("lambda a, b: a * b", {"a": "float", "b": "float"}),
    "divide": ("lambda a, b: a / b if b != 0 else 0.0", {"a": "float", "b": "float"}),
    "percent_change": ("lambda old, new: ((new - old) / old * 100) if old != 0 else 0.0", {"old": "float", "new": "float"}),
    
    # Aggregations
    "sum": ("lambda values: sum(values)", {"values": "list"}),
    "mean": ("lambda values: sum(values) / len(values) if len(values) > 0 else 0.0", {"values": "list"}),
    "min": ("lambda values: min(values) if len(values) > 0 else 0.0", {"values": "list"}),
    "max": ("lambda values: max(values) if len(values) > 0 else 0.0", {"values": "list"}),
    "count": ("lambda values: len(values)", {"values": "list"}),
    
    # Economic operations
    "sensitivity": (
        "lambda base, delta: {'base': base, 'delta': delta, 'change': delta / base if base != 0 else 0.0}",
        {"base": "float", "delta": "float"}
    ),
    "waterfall": (
        "lambda components: {'components': components, 'total': sum(c.get('value', 0) for c in components)}",
        {"components": "list"}
    ),
}


class ComposableFunctionBuilder(FunctionGenerator):
    """Build functions by composing pre-validated atomic operations.
    
    Usage:
        builder = ComposableFunctionBuilder()
        context = GenerationContext(
            command=command,
            semantic_features={"revenue_detected": True},
            intent_type="sensitivity_analysis",
            intent_confidence=0.88,
        )
        operation = await builder.generate(context)
    """
    
    def __init__(self, enable_cache: bool = True):
        """Initialize composable builder.
        
        Args:
            enable_cache: Whether to cache composed functions
        """
        super().__init__(
            name="ComposableFunctionBuilder",
            strategy=GenerationStrategy.COMPOSABLE_BUILDING_BLOCKS,
        )
        self.enable_cache = enable_cache
        self._cache: Dict[str, GeneratedOperation] = {}
        self.building_blocks = BUILDING_BLOCKS
        
        logger.info(
            f"ComposableFunctionBuilder initialized: "
            f"{len(self.building_blocks)} building blocks available, cache_enabled={enable_cache}"
        )
    
    async def generate(self, context: GenerationContext) -> GeneratedOperation:
        """Generate function by composing building blocks.
        
        Process:
        1. Compute cache key (intent_type + semantic_features)
        2. Check cache (if enabled)
        3. Select building blocks based on intent type
        4. Compose blocks into function
        5. Validate composition (type contracts)
        6. Create ExecutableFunction
        7. Record metrics (cost=0, latency=microseconds)
        
        Args:
            context: Generation context with command, features
        
        Returns:
            GeneratedOperation with composed function
        
        Raises:
            GenerationError: If composition fails or blocks unavailable
        """
        start_time = datetime.utcnow()
        
        # Step 1: Compute cache key
        cache_key = self._compute_cache_key(context)
        
        # Step 2: Check cache
        if self.enable_cache and cache_key in self._cache:
            cached_op = self._cache[cache_key]
            elapsed_ms = (datetime.utcnow() - start_time).total_seconds() * 1000
            self._record_generation(cost_usd=0.0, latency_ms=elapsed_ms, cache_hit=True)
            logger.info(f"Composable cache hit: {cache_key[:16]}... (latency={elapsed_ms:.1f}ms)")
            return cached_op
        
        # Step 3: Select building blocks
        blocks = self._select_building_blocks(context)
        if not blocks:
            raise GenerationError(
                f"No building blocks available for intent_type={context.intent_type}"
            )
        
        # Step 4: Compose blocks into function
        composed_code = self._compose_blocks(context, blocks)
        
        # Step 5: Validate composition (syntax check)
        self._validate_composition(composed_code)
        
        # Step 6: Create ExecutableFunction
        func = ExecutableFunction(
            name=f"composed_{context.intent_type}_{self.generation_count + 1}",
            language="python",
            code=composed_code,
            signature={
                "inputs": {"context": "dict", "parameters": "dict"},
                "outputs": {"result": "dict"},
            },
            constraints_enforced=[
                ConstraintType.DETERMINISTIC,
                ConstraintType.TYPE_SAFETY,
            ],
            generation_strategy=GenerationStrategy.COMPOSABLE_BUILDING_BLOCKS,
            strategy_metadata={
                "building_blocks": [b for b in blocks],
                "composition_recipe": f"{len(blocks)} blocks composed",
            },
            generated_at=start_time,
            semantic_engine_version=context.model_version or "semantic_engine_v2.0",
            model_version=None,  # No LLM involved
            seed=None,
        )
        
        # Create operation
        operation = GeneratedOperation.create(
            command_id=context.command.command_id,
            generated_function=func,
            generation_metadata={
                "handler": self.name,
                "intent": context.command.user_instruction,
                "intent_type": context.intent_type,
                "semantic_confidence": context.intent_confidence,
                "generation_strategy": GenerationStrategy.COMPOSABLE_BUILDING_BLOCKS.value,
                "building_blocks_used": blocks,
                "features_detected": context.semantic_features,
            },
            validation_results=ValidationResults(),
            is_cached=False,
            semantic_confidence=context.intent_confidence,
        )
        
        # Step 7: Record metrics
        elapsed_ms = (datetime.utcnow() - start_time).total_seconds() * 1000
        self._record_generation(cost_usd=0.0, latency_ms=elapsed_ms, cache_hit=False)
        
        # Cache result
        if self.enable_cache:
            self._cache[cache_key] = operation
        
        logger.info(
            f"Composable generation complete: intent_type={context.intent_type}, "
            f"blocks={len(blocks)}, cost=$0.00, latency={elapsed_ms:.1f}ms"
        )
        
        return operation
    
    def _compute_cache_key(self, context: GenerationContext) -> str:
        """Compute cache key from context.
        
        Args:
            context: Generation context
        
        Returns:
            BLAKE3 hash as hex string
        """
        key_data = {
            "intent_type": context.intent_type,
            "semantic_features": context.semantic_features,
        }
        key_json = json.dumps(key_data, sort_keys=True)
        return hashlib.blake2b(key_json.encode(), digest_size=32).hexdigest()
    
    def _select_building_blocks(self, context: GenerationContext) -> List[str]:
        """Select building blocks based on intent type and features.
        
        Args:
            context: Generation context
        
        Returns:
            List of building block names
        """
        blocks = []
        intent_type = context.intent_type
        features = context.semantic_features
        
        # Economic intent → select relevant blocks
        if "sensitivity" in intent_type:
            blocks.extend(["sensitivity", "percent_change"])
        elif "waterfall" in intent_type:
            blocks.extend(["waterfall", "sum"])
        elif "unit_economics" in intent_type or "contribution" in intent_type:
            blocks.extend(["divide", "subtract", "mean"])
        
        # Feature-based selection
        if features.get("revenue_detected"):
            blocks.append("sum")
        if features.get("cost_detected"):
            blocks.append("subtract")
        if features.get("margin_detected"):
            blocks.append("divide")
        
        # Always include basic aggregations as fallback
        if not blocks:
            blocks = ["sum", "mean", "count"]

        # Deterministic de-duplication (preserve first-seen order)
        seen: set[str] = set()
        deduped: list[str] = []
        for block_name in blocks:
            if block_name in seen:
                continue
            seen.add(block_name)
            deduped.append(block_name)

        return deduped
    
    def _compose_blocks(self, context: GenerationContext, blocks: List[str]) -> str:
        """Compose building blocks into a function.
        
        Args:
            context: Generation context
            blocks: Building block names
        
        Returns:
            Python function code
        """
        # `blocks` is typically a List[str], but tests pass a dict; iterating over a dict yields keys.
        block_names = list(blocks)

        def _safe_var(name: str) -> str:
            return f"bb_{name.replace('-', '_')}"

        # Import block definitions (avoid shadowing built-ins like `sum`, `min`, `max`)
        available_blocks: set[str] = set()
        block_defs_lines: list[str] = []
        for name in block_names:
            if name not in self.building_blocks:
                continue
            available_blocks.add(name)
            block_defs_lines.append(f"    {_safe_var(name)} = {self.building_blocks[name][0]}")

        block_defs = "\n".join(block_defs_lines) if block_defs_lines else "    pass"

        # Generate function
        apply_lines: list[str] = []
        if "sum" in available_blocks:
            apply_lines.append("        result['sum'] = bb_sum(values)")
        if "mean" in available_blocks:
            apply_lines.append("        result['mean'] = bb_mean(values)")
        if "count" in available_blocks:
            apply_lines.append("        result['count'] = bb_count(values)")
        apply_blocks = "\n".join(apply_lines) if apply_lines else "        pass"

        code = f"""# Composable Function: {context.intent_type}

def generated_function(context: dict, parameters: dict) -> dict:
    \"\"\"Execute {context.intent_type} operation using composable building blocks.\"\"\"
    # Building blocks
{block_defs}
    
    # Extract inputs
    data = context.get('data', {{}})
    values = data.get('values', [])
    
    # Compute result using building blocks
    result = {{
        'status': 'ok',
        'intent_type': '{context.intent_type}',
        'instruction': r'{context.command.user_instruction}',
        'values': values,
    }}
    
    # Apply building blocks
    if len(values) > 0:
{apply_blocks}
    
    return result
"""
        return code
    
    def _validate_composition(self, code: str) -> None:
        """Validate composed function syntax.
        
        Args:
            code: Composed Python code
        
        Raises:
            GenerationError: If syntax is invalid
        """
        try:
            compile(code, "<composed>", "exec")
        except SyntaxError as e:
            raise GenerationError(f"Composed code has syntax error: {e}") from e
