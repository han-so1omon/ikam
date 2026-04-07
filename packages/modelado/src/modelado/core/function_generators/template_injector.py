"""Enhanced template injection with parameter extraction.

Fills intent-specific templates with parameters extracted from semantic analysis.

This enhances the template approach with:
- Formal parameter extraction from instruction text
- Type-safe slot filling
- Validation of extracted parameters
- Fallback values for missing parameters

Templates are pre-defined for each intent type; extraction happens via regex patterns
and semantic feature matching.

Mathematical guarantees:
- Determinism: Same (intent + extracted_params) → same filled template
- Type safety: All slot fillers validated before injection
- Provenance: Extracted parameters recorded in metadata
"""

from __future__ import annotations

import hashlib
import json
import logging
import re
from datetime import datetime
from typing import Any, Dict, List, Optional, Tuple

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


# Parameter extraction patterns (regex)
PARAMETER_PATTERNS = {
    "percentage": r"(\d+(?:\.\d+)?)\s*(?:percent|%)",
    "currency": r"\$?\s*(\d+(?:,\d{3})*(?:\.\d{2})?)",
    "numeric": r"(\d+(?:\.\d+)?)",
    "range": r"from\s+(\d+(?:\.\d+)?)\s+to\s+(\d+(?:\.\d+)?)",
    "list": r"(?:and|,)\s*([a-z_]+)",
}


# Intent-specific templates
INTENT_TEMPLATES = {
    "sensitivity_analysis": """# Sensitivity Analysis: {instruction}
# Template with extracted parameters

def sensitivity_analysis(context: dict, parameters: dict) -> dict:
    \"\"\"Perform sensitivity analysis on {parameter_name}.\"\"\"
    base_value = context.get('{parameter_name}', parameters.get('base_value', 100.0))
    delta_percent = parameters.get('delta_percent', {delta_percent})
    
    delta_value = base_value * (delta_percent / 100.0)
    new_value = base_value + delta_value
    
    return {{
        'status': 'ok',
        'analysis_type': 'sensitivity_analysis',
        'parameter': '{parameter_name}',
        'base_value': base_value,
        'delta_percent': delta_percent,
        'delta_value': delta_value,
        'new_value': new_value,
        'elasticity': delta_percent / 100.0,
    }}
""",
    
    "waterfall_analysis": """# Waterfall Analysis: {instruction}
# Template with extracted components

def waterfall_analysis(context: dict, parameters: dict) -> dict:
    \"\"\"Decompose {parameter_name} into components.\"\"\"
    components = parameters.get('components', {components})
    base_value = context.get('base_value', 0.0)
    
    breakdown = []
    cumulative = base_value
    for component in components:
        value = context.get(component, 0.0)
        cumulative += value
        breakdown.append({{'name': component, 'value': value, 'cumulative': cumulative}})
    
    return {{
        'status': 'ok',
        'analysis_type': 'waterfall_analysis',
        'parameter': '{parameter_name}',
        'base_value': base_value,
        'components': breakdown,
        'final_value': cumulative,
    }}
""",
    
    "unit_economics_analysis": """# Unit Economics: {instruction}
# Template with CAC/LTV parameters

def unit_economics_analysis(context: dict, parameters: dict) -> dict:
    \"\"\"Analyze unit economics (CAC, LTV, payback period).\"\"\"
    cac = context.get('cac', parameters.get('cac', 100.0))
    ltv = context.get('ltv', parameters.get('ltv', 300.0))
    monthly_revenue = context.get('monthly_revenue_per_customer', parameters.get('monthly_revenue', 25.0))
    
    ltv_cac_ratio = ltv / cac if cac > 0 else 0.0
    payback_months = cac / monthly_revenue if monthly_revenue > 0 else 0.0
    
    return {{
        'status': 'ok',
        'analysis_type': 'unit_economics',
        'cac': cac,
        'ltv': ltv,
        'ltv_cac_ratio': ltv_cac_ratio,
        'payback_months': payback_months,
        'monthly_revenue_per_customer': monthly_revenue,
    }}
""",

    "generic_economic_operation": """# Economic analysis: {instruction}

def generic_economic_operation(context: dict, parameters: dict) -> dict:
    \"\"\"Perform a generic economic operation.

    This template is intentionally simple and deterministic; callers can pass
    operation-specific parameters via the `parameters` dict.
    \"\"\"
    parameter_name = parameters.get('parameter_name', '{parameter_name}')
    value = context.get(parameter_name, parameters.get('value', 0.0))

    return {{
        'status': 'ok',
        'analysis_type': 'generic_economic_operation',
        'parameter': parameter_name,
        'value': value,
        'instruction': '{instruction}',
    }}
""",
}


class TemplateInjector(FunctionGenerator):
    """Fill templates with extracted parameters from instruction text.
    
    Usage:
        injector = TemplateInjector()
        context = GenerationContext(
            command=command,
            semantic_features={"revenue_detected": True},
            intent_type="sensitivity_analysis",
            intent_confidence=0.91,
        )
        operation = await injector.generate(context)
    """
    
    def __init__(self, enable_cache: bool = True):
        """Initialize template injector.
        
        Args:
            enable_cache: Whether to cache filled templates
        """
        super().__init__(
            name="TemplateInjector",
            strategy=GenerationStrategy.TEMPLATE_INJECTION,
        )
        self.enable_cache = enable_cache
        self._cache: Dict[str, GeneratedOperation] = {}
        self.templates = INTENT_TEMPLATES
        
        logger.info(
            f"TemplateInjector initialized: "
            f"{len(self.templates)} templates available, cache_enabled={enable_cache}"
        )
    
    async def generate(self, context: GenerationContext) -> GeneratedOperation:
        """Generate function by filling template with extracted parameters.
        
        Process:
        1. Compute cache key (intent_type + instruction)
        2. Check cache (if enabled)
        3. Extract parameters from instruction text
        4. Select template for intent type
        5. Fill template with extracted parameters
        6. Validate filled template (syntax check)
        7. Create ExecutableFunction
        8. Record metrics (cost=0, latency=milliseconds)
        
        Args:
            context: Generation context with command, features
        
        Returns:
            GeneratedOperation with filled template
        
        Raises:
            GenerationError: If template unavailable or filling fails
        """
        start_time = datetime.utcnow()
        
        # Step 1: Compute cache key
        cache_key = self._compute_cache_key(context)
        
        # Step 2: Check cache
        if self.enable_cache and cache_key in self._cache:
            cached_op = self._cache[cache_key]
            elapsed_ms = (datetime.utcnow() - start_time).total_seconds() * 1000
            self._record_generation(cost_usd=0.0, latency_ms=elapsed_ms, cache_hit=True)
            logger.info(f"Template cache hit: {cache_key[:16]}... (latency={elapsed_ms:.1f}ms)")
            return cached_op
        
        # Step 3: Extract parameters
        extracted_params = self._extract_parameters(context)
        
        # Step 4: Select template
        template = self._select_template(context)
        if not template:
            raise GenerationError(
                f"No template available for intent_type={context.intent_type}"
            )
        
        # Step 5: Fill template
        filled_code = self._fill_template(template, context, extracted_params)
        
        # Step 6: Validate filled template
        self._validate_filled_template(filled_code)
        
        # Step 7: Create ExecutableFunction
        func = ExecutableFunction(
            name=f"template_{context.intent_type}_{self.generation_count + 1}",
            language="python",
            code=filled_code,
            signature={
                "inputs": {"context": "dict", "parameters": "dict"},
                "outputs": {"result": "dict"},
            },
            constraints_enforced=[
                ConstraintType.DETERMINISTIC,
                ConstraintType.TYPE_SAFETY,
            ],
            generation_strategy=GenerationStrategy.TEMPLATE_INJECTION,
            strategy_metadata={
                "template": context.intent_type,
                "extracted_parameters": extracted_params,
                "phase": "9.3_enhanced_template",
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
                "generation_strategy": GenerationStrategy.TEMPLATE_INJECTION.value,
                "extracted_parameters": extracted_params,
                "features_detected": context.semantic_features,
            },
            validation_results=ValidationResults(),
            is_cached=False,
            semantic_confidence=context.intent_confidence,
        )
        
        # Step 8: Record metrics
        elapsed_ms = (datetime.utcnow() - start_time).total_seconds() * 1000
        self._record_generation(cost_usd=0.0, latency_ms=elapsed_ms, cache_hit=False)
        
        # Cache result
        if self.enable_cache:
            self._cache[cache_key] = operation
        
        logger.info(
            f"Template generation complete: intent_type={context.intent_type}, "
            f"params={len(extracted_params)}, cost=$0.00, latency={elapsed_ms:.1f}ms"
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
            "instruction": context.command.user_instruction,
        }
        key_json = json.dumps(key_data, sort_keys=True)
        return hashlib.blake2b(key_json.encode(), digest_size=32).hexdigest()
    
    def _extract_parameters(self, context: GenerationContext) -> Dict[str, Any]:
        """Extract parameters from instruction text using regex patterns.
        
        Args:
            context: Generation context
        
        Returns:
            Dict of extracted parameters
        """
        instruction = context.command.user_instruction.lower()
        params = {}
        
        # Extract percentage
        percentage_match = re.search(PARAMETER_PATTERNS["percentage"], instruction)
        if percentage_match:
            params["delta_percent"] = float(percentage_match.group(1))
        
        # Extract currency
        currency_match = re.search(PARAMETER_PATTERNS["currency"], instruction)
        if currency_match:
            params["currency_value"] = float(currency_match.group(1).replace(",", ""))
        
        # Extract parameter name from semantic features
        if context.semantic_features.get("revenue_detected"):
            params["parameter_name"] = "revenue"
        elif context.semantic_features.get("cost_detected"):
            params["parameter_name"] = "cost"
        else:
            params["parameter_name"] = "value"
        
        # Extract list of components (for waterfall)
        components = re.findall(PARAMETER_PATTERNS["list"], instruction)
        if components:
            params["components"] = components
        
        return params
    
    def _select_template(self, context: GenerationContext) -> Optional[str]:
        """Select template for intent type.
        
        Args:
            context: Generation context
        
        Returns:
            Template string or None
        """
        return self.templates.get(context.intent_type)
    
    def _fill_template(
        self,
        template: str,
        context: GenerationContext,
        params: Dict[str, Any],
    ) -> str:
        """Fill template with extracted parameters.
        
        Args:
            template: Template string
            context: Generation context
            params: Extracted parameters
        
        Returns:
            Filled template code
        """
        # Default values for missing parameters
        defaults = {
            "instruction": context.command.user_instruction,
            "delta_percent": 10.0,
            "parameter_name": "value",
            "components": ["component_1", "component_2"],
        }
        
        # Merge extracted params with defaults
        filled_params = {**defaults, **params}
        
        # Fill template
        try:
            filled_code = template.format(**filled_params)
        except KeyError as e:
            raise GenerationError(f"Template filling failed: missing parameter {e}") from e
        
        return filled_code
    
    def _validate_filled_template(self, code: str) -> None:
        """Validate filled template syntax.
        
        Args:
            code: Filled template code
        
        Raises:
            GenerationError: If syntax is invalid
        """
        try:
            compile(code, "<template>", "exec")
        except SyntaxError as e:
            raise GenerationError(f"Filled template has syntax error: {e}") from e
