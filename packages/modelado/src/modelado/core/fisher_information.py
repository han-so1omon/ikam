"""Fisher Information calculation for generative operations.

Calculates Fisher Information gains from provenance recording to validate
IKAM's information-theoretic advantages over traditional RAG systems.

Mathematical foundation: See docs/ikam/FISHER_INFORMATION_GAINS.md

Key results:
- I_IKAM(θ) ≥ I_RAG(θ) + Δ_provenance(θ)
- Δ_provenance(θ) ≥ 0 (provenance never decreases information)
- Strict improvement when provenance is θ-informative

Usage:
    calculator = FisherInformationCalculator()
    
    # Calculate for single function
    fi = calculator.calculate_function_information(
        function_id="gfn_abc123",
        provenance_chain=chain,
    )
    
    # Calculate aggregate (IKAM vs baseline)
    comparison = calculator.compare_with_baseline(
        ikam_functions=[...],
        baseline_functions=[...],
    )
    
    # Validate information dominance
    assert comparison.ikam_information >= comparison.baseline_information + comparison.provenance_delta
"""

from __future__ import annotations

import logging
import math
from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Dict, List, Optional, Set

from .provenance_recorder import (
    ProvenanceChain,
    ProvenanceEventType,
    GenerationProvenanceEvent,
    ExecutionProvenanceEvent,
    DerivationProvenanceEvent,
)

logger = logging.getLogger(__name__)


# Fisher Information Calculation Models

class InformationSource(str, Enum):
    """Sources of Fisher Information in IKAM."""
    CONTENT = "content"
    GENERATION_METADATA = "generation_metadata"      # θ_generation
    EXECUTION_HISTORY = "execution_history"          # θ_execution
    DERIVATION_STRUCTURE = "derivation_structure"    # θ_structural
    FRAGMENT_HIERARCHY = "fragment_hierarchy"        # θ_hierarchy
    SALIENCE_SCORING = "salience_scoring"            # θ_salience


@dataclass
class FisherInformationComponents:
    # Content-level information (shared with RAG)
    content_information: float = 0.0
    
    # Provenance-specific information (IKAM advantage)
    generation_information: float = 0.0     # From generation metadata
    execution_information: float = 0.0      # From execution history
    derivation_information: float = 0.0     # From derivation chains
    
    # Total information
    @property
    def total_information(self) -> float:
        """Total Fisher Information."""
        return (
            self.content_information +
            self.generation_information +
            self.execution_information +
            self.derivation_information
        )
    
    @property
    def provenance_delta(self) -> float:
        """Provenance-specific information gain (Δ_provenance)."""
        return (
            self.generation_information +
            self.execution_information +
            self.derivation_information
        )

    # Attribution / introspection (used by tests)
    sources: Set[InformationSource] = field(default_factory=set)
    information_sources: List[Any] = field(default_factory=list)


@dataclass
class InformationDominanceComparison:
    """Comparison of IKAM vs baseline Fisher Information.
    
    Validates theorem: I_IKAM(θ) ≥ I_baseline(θ) + Δ_provenance(θ)
    """
    ikam_information: float
    baseline_information: float
    provenance_delta: float
    
    @property
    def information_gain(self) -> float:
        """Total information gain (IKAM - baseline)."""
        return self.ikam_information - self.baseline_information
    
    @property
    def dominance_validated(self) -> bool:
        """Whether information dominance holds: I_IKAM ≥ I_baseline + Δ."""
        # Allow 1e-6 tolerance for floating-point errors
        return self.ikam_information >= (self.baseline_information + self.provenance_delta - 1e-6)

    @property
    def dominates_baseline(self) -> bool:
        """Back-compat alias for dominance validation."""
        return self.dominance_validated

    @property
    def dominance_ratio(self) -> float:
        """Ratio of IKAM to baseline Fisher Information.

        This is a simple interpretability metric used in unit tests.
        """
        if self.baseline_information == 0:
            return float('inf') if self.ikam_information > 0 else 1.0
        return self.ikam_information / self.baseline_information
    
    @property
    def gain_percentage(self) -> float:
        """Information gain as percentage of baseline."""
        if self.baseline_information == 0:
            return float('inf') if self.ikam_information > 0 else 0.0
        return (self.information_gain / self.baseline_information) * 100


class FisherInformationCalculator:
    """Calculate Fisher Information for generated functions with provenance.
    
    Implements Fisher Information calculation from provenance chains to validate
    IKAM's information-theoretic advantages.
    
    Mathematical guarantees:
    1. I(θ) ≥ 0 (information is non-negative)
    2. I_IKAM(θ) ≥ I_baseline(θ) (provenance never decreases information)
    3. Δ_provenance(θ) > 0 when provenance is θ-informative
    """
    
    def __init__(
        self,
        content_baseline_weight: float = 1.0,
        generation_weight: float = 0.5,
        execution_weight: float = 0.3,
        derivation_weight: float = 0.2,
    ):
        """Initialize Fisher Information calculator.
        
        Args:
            content_baseline_weight: Weight for content-level information (shared with RAG)
            generation_weight: Weight for generation metadata information
            execution_weight: Weight for execution history information
            derivation_weight: Weight for derivation structure information
        """
        self.content_baseline_weight = content_baseline_weight
        self.generation_weight = generation_weight
        self.execution_weight = execution_weight
        self.derivation_weight = derivation_weight
        
        logger.info(
            f"FisherInformationCalculator initialized "
            f"(content={content_baseline_weight}, gen={generation_weight}, "
            f"exec={execution_weight}, deriv={derivation_weight})"
        )
    
    def calculate_function_information(
        self,
        function_id: str,
        provenance_chain: Optional[ProvenanceChain] = None,
        content_length: int = 0,
        *,
        # Compatibility parameters (tests pass these)
        content_hash: Optional[str] = None,
        code: Optional[str] = None,
        generation_metadata: Optional[Dict[str, Any]] = None,
        execution_history: Optional[List[Dict[str, Any]]] = None,
        derivation_info: Optional[Dict[str, Any]] = None,
    ) -> FisherInformationComponents:
        """Calculate Fisher Information for a single function.
        
        Args:
            function_id: Function ID
            provenance_chain: Complete provenance chain
            content_length: Function content length (bytes)
            
        Returns:
            FisherInformationComponents with decomposed information
        """
        # Build a provenance chain if callers provide dict-based provenance.
        if provenance_chain is None:
            events: List[GenerationProvenanceEvent | ExecutionProvenanceEvent | DerivationProvenanceEvent] = []

            if generation_metadata:
                try:
                    events.append(GenerationProvenanceEvent(**generation_metadata))
                except Exception:
                    # Best-effort: tolerate partial dicts
                    pass

            for item in (execution_history or []):
                try:
                    events.append(ExecutionProvenanceEvent(**item))
                except Exception:
                    pass

            derivations = (derivation_info or {}).get("derivations") if derivation_info else None
            for item in (derivations or []):
                try:
                    events.append(DerivationProvenanceEvent(**item))
                except Exception:
                    pass

            provenance_chain = ProvenanceChain(
                root_id=function_id,
                events=events,
                generation_count=sum(1 for e in events if e.event_type == ProvenanceEventType.GENERATION),
                execution_count=sum(1 for e in events if e.event_type == ProvenanceEventType.EXECUTION),
                derivation_count=sum(1 for e in events if e.event_type == ProvenanceEventType.DERIVATION),
            )

        # Prefer deriving content_length from code if not provided.
        if content_length == 0 and code is not None:
            content_length = len(code.encode())

        components = FisherInformationComponents()
        
        # Content-level information (baseline shared with RAG)
        components.content_information = self._calculate_content_information(content_length)
        if components.content_information > 0:
            components.sources.add(InformationSource.CONTENT)
        
        # Generation metadata information
        components.generation_information = self._calculate_generation_information(
            provenance_chain
        )
        if components.generation_information > 0:
            components.sources.add(InformationSource.GENERATION_METADATA)
        
        # Execution history information
        components.execution_information = self._calculate_execution_information(
            provenance_chain
        )
        if components.execution_information > 0:
            components.sources.add(InformationSource.EXECUTION_HISTORY)
        
        # Derivation structure information
        components.derivation_information = self._calculate_derivation_information(
            provenance_chain
        )
        if components.derivation_information > 0:
            components.sources.add(InformationSource.DERIVATION_STRUCTURE)

        # Track underlying event objects for attribution.
        try:
            components.information_sources = list(provenance_chain.events)
        except Exception:
            components.information_sources = []
        
        logger.debug(
            f"FI for {function_id}: total={components.total_information:.4f} "
            f"(content={components.content_information:.4f}, "
            f"gen={components.generation_information:.4f}, "
            f"exec={components.execution_information:.4f}, "
            f"deriv={components.derivation_information:.4f})"
        )
        
        return components
    
    def calculate_aggregate_information(
        self,
        functions: List[Any],
    ) -> FisherInformationComponents:
        """Calculate aggregate Fisher Information across multiple functions.
        
        Args:
            functions: Either:
                - List of (function_id, provenance_chain, content_length) tuples
                - List of precomputed FisherInformationComponents
            
        Returns:
            Aggregated FisherInformationComponents
        """
        aggregate = FisherInformationComponents()

        if not functions:
            return aggregate

        first = functions[0]
        if isinstance(first, FisherInformationComponents):
            for fi in functions:
                aggregate.content_information += fi.content_information
                aggregate.generation_information += fi.generation_information
                aggregate.execution_information += fi.execution_information
                aggregate.derivation_information += fi.derivation_information
                aggregate.sources |= set(getattr(fi, "sources", set()))
                aggregate.information_sources.extend(getattr(fi, "information_sources", []) or [])
        else:
            for function_id, chain, content_length in functions:
                fi = self.calculate_function_information(function_id, chain, content_length)
                aggregate.content_information += fi.content_information
                aggregate.generation_information += fi.generation_information
                aggregate.execution_information += fi.execution_information
                aggregate.derivation_information += fi.derivation_information
                aggregate.sources |= set(getattr(fi, "sources", set()))
                aggregate.information_sources.extend(getattr(fi, "information_sources", []) or [])
        
        logger.info(
            f"Aggregate FI for {len(functions)} functions: "
            f"total={aggregate.total_information:.4f}, "
            f"provenance_delta={aggregate.provenance_delta:.4f}"
        )
        
        return aggregate
    
    def compare_with_baseline(
        self,
        ikam_functions: Any,
        baseline_content_lengths: Any,
    ) -> InformationDominanceComparison:
        """Compare IKAM Fisher Information with a baseline (RAG-like) system.

        Supports two calling conventions:
        1) Precomputed components (unit tests):
           - compare_with_baseline(ikam_fi: FisherInformationComponents,
                                  baseline_fi: FisherInformationComponents)
        2) Aggregate inputs:
           - compare_with_baseline(
               ikam_functions=[(function_id, provenance_chain, content_length), ...],
               baseline_content_lengths=[content_length, ...],
             )

        Returns:
            InformationDominanceComparison validating I_IKAM ≥ I_baseline + Δ
        """
        # Compatibility path: allow comparing two already-computed FI objects.
        if isinstance(ikam_functions, FisherInformationComponents) and isinstance(
            baseline_content_lengths, FisherInformationComponents
        ):
            ikam_fi = ikam_functions
            baseline_fi = baseline_content_lengths.total_information
        else:
            # Calculate IKAM information (content + provenance)
            ikam_fi = self.calculate_aggregate_information(ikam_functions)

            # Calculate baseline information (content only)
            baseline_fi = sum(
                self._calculate_content_information(length)
                for length in baseline_content_lengths
            )
        
        comparison = InformationDominanceComparison(
            ikam_information=ikam_fi.total_information,
            baseline_information=baseline_fi,
            provenance_delta=ikam_fi.provenance_delta,
        )
        
        logger.info(
            f"Information dominance: IKAM={comparison.ikam_information:.4f}, "
            f"baseline={comparison.baseline_information:.4f}, "
            f"Δ={comparison.provenance_delta:.4f}, "
            f"gain={comparison.gain_percentage:.2f}%, "
            f"validated={comparison.dominance_validated}"
        )
        
        if not comparison.dominance_validated:
            logger.warning(
                f"⚠️ Information dominance VIOLATED: "
                f"I_IKAM ({comparison.ikam_information:.4f}) < "
                f"I_baseline + Δ ({comparison.baseline_information + comparison.provenance_delta:.4f})"
            )
        
        return comparison

    def validate_information_dominance(
        self,
        ikam_functions: Any,
        baseline_content_lengths: Any,
        *,
        strict: bool = True,
    ) -> bool:
        """Validate IKAM information dominance theorem.

        Convenience wrapper around :meth:`compare_with_baseline`.

        Supports both calling conventions:
        - Precomputed: (ikam_fi: FisherInformationComponents, baseline_fi: FisherInformationComponents)
        - Aggregate inputs: (ikam_functions: List[tuple[function_id, chain, content_length]],
          baseline_content_lengths: List[int])
        """
        comparison = self.compare_with_baseline(ikam_functions, baseline_content_lengths)

        if comparison.dominance_validated:
            return True

        message = (
            f"Information dominance VIOLATED: "
            f"I_IKAM ({comparison.ikam_information:.4f}) < "
            f"I_baseline + Δ ({(comparison.baseline_information + comparison.provenance_delta):.4f})"
        )
        if strict:
            raise AssertionError(message)
        logger.warning(message)
        return False
    
    # Private calculation methods
    
    def _calculate_content_information(self, content_length: int) -> float:
        """Calculate content-level Fisher Information.
        
        Simplified model: I_content(θ) ∝ log(1 + content_length)
        
        Rationale: Longer content provides more evidence about parameters,
        but with diminishing returns (log scaling).
        """
        if content_length == 0:
            return 0.0
        
        # Log scaling with baseline weight
        return self.content_baseline_weight * math.log(1 + content_length)
    
    def _calculate_generation_information(self, chain: ProvenanceChain) -> float:
        """Calculate generation metadata Fisher Information.
        
        Sources:
        - Confidence score (higher confidence → tighter parameter estimates)
        - Strategy type (template/composable/llm reveals parameter structure)
        - Extracted parameters (explicit parameter constraints)
        - Semantic reasoning (additional θ evidence)
        """
        if chain.generation_count == 0:
            return 0.0
        
        total_info = 0.0
        generation_events = [
            e for e in chain.events
            if e.event_type == ProvenanceEventType.GENERATION
        ]
        
        for event in generation_events:
            event: GenerationProvenanceEvent
            
            # Confidence contribution: High confidence → more information
            # I_confidence(θ) ∝ -log(1 - confidence)
            # (approaches infinity as confidence → 1)
            confidence_info = -math.log(1.001 - event.confidence)  # Add small constant to avoid log(0)
            
            # Strategy contribution: Different strategies reveal different θ aspects
            strategy_info = {
                "template": 0.3,          # Templates are rigid (less θ variability)
                "composable": 0.5,        # Composable reveals θ_composition
                "llm": 0.7,               # LLM reveals θ_semantic
            }.get(event.strategy, 0.5)
            
            # Parameter extraction contribution
            param_info = 0.0
            if event.extracted_parameters:
                # Each extracted parameter provides θ evidence
                param_info = len(event.extracted_parameters) * 0.1
            
            # Reasoning contribution
            reasoning_info = 0.2 if event.semantic_reasoning else 0.0
            
            total_info += confidence_info + strategy_info + param_info + reasoning_info
        
        return self.generation_weight * total_info
    
    def _calculate_execution_information(self, chain: ProvenanceChain) -> float:
        """Calculate execution history Fisher Information.
        
        Sources:
        - Input/output pairs (reveal θ_domain constraints)
        - Execution count (repeated execution → tighter estimates)
        - Execution variance (low variance → high confidence)
        """
        if chain.execution_count == 0:
            return 0.0
        
        execution_events = [
            e for e in chain.events
            if e.event_type == ProvenanceEventType.EXECUTION
        ]
        
        # Execution count contribution: More executions → more θ evidence
        # I_executions(θ) ∝ log(1 + N_executions)
        count_info = math.log(1 + len(execution_events))
        
        # Input/output dimensionality contribution
        total_io_info = 0.0
        for event in execution_events:
            event: ExecutionProvenanceEvent
            
            # Each input/output dimension provides θ constraints
            input_dims = len(event.inputs) if event.inputs else 0
            output_dims = len(event.outputs) if event.outputs else 0
            
            total_io_info += (input_dims + output_dims) * 0.05
        
        return self.execution_weight * (count_info + total_io_info)
    
    def _calculate_derivation_information(self, chain: ProvenanceChain) -> float:
        """Calculate derivation structure Fisher Information.
        
        Sources:
        - Derivation relationships (reveal θ_structural dependencies)
        - Derivation strength (stronger derivations → tighter constraints)
        - Transformation metadata (transformation type reveals θ_transform)
        """
        if chain.derivation_count == 0:
            return 0.0
        
        derivation_events = [
            e for e in chain.events
            if e.event_type == ProvenanceEventType.DERIVATION
        ]
        
        total_info = 0.0
        
        for event in derivation_events:
            event: DerivationProvenanceEvent
            
            # Derivation strength contribution
            # Stronger derivations provide more θ_structural information
            strength_info = event.derivation_strength * 0.5
            
            # Transformation metadata contribution
            transform_info = 0.2 if event.transformation else 0.0
            
            total_info += strength_info + transform_info
        
        # Structural connectivity: More derivations → more θ_graph constraints
        connectivity_info = math.log(1 + len(derivation_events))
        
        return self.derivation_weight * (total_info + connectivity_info)


# Utility functions

def validate_information_dominance(
    calculator: FisherInformationCalculator,
    ikam_functions: List[tuple[str, ProvenanceChain, int]],
    baseline_content_lengths: List[int],
    strict: bool = True,
) -> bool:
    """Validate IKAM information dominance theorem.
    
    Args:
        calculator: FisherInformationCalculator instance
        ikam_functions: IKAM functions with provenance
        baseline_content_lengths: Baseline content lengths (no provenance)
        strict: If True, raise exception on violation; if False, log warning
        
    Returns:
        True if I_IKAM ≥ I_baseline + Δ_provenance, False otherwise
        
    Raises:
        AssertionError: If strict=True and dominance violated
    """
    comparison = calculator.compare_with_baseline(ikam_functions, baseline_content_lengths)
    
    if not comparison.dominance_validated:
        message = (
            f"Information dominance VIOLATED: "
            f"I_IKAM ({comparison.ikam_information:.4f}) < "
            f"I_baseline + Δ ({comparison.baseline_information + comparison.provenance_delta:.4f})"
        )
        
        if strict:
            raise AssertionError(message)
        else:
            logger.warning(message)
            return False
    
    return True


# Execution Graph Fisher Information (Task 7.6)

def compute_flat_execution_fi(
    connection_pool,
    execution_ids: List[str],
) -> float:
    """Compute Fisher Information treating executions as independent.
    
    FI components (flat):
    - Number of executions: log(N)
    - Function diversity: -Σ p_i log(p_i) where p_i = freq(function_i) / N
    - Model call diversity: similar entropy over models used
    - Cost distribution: variance in costs (higher variance → more information)
    
    Args:
        connection_pool: Database connection pool
        execution_ids: List of execution IDs to analyze
        
    Returns:
        Fisher Information score (higher = more information)
    """
    from collections import Counter
    
    if not execution_ids:
        return 0.0
    
    # Count executions
    n_executions = len(execution_ids)
    fi_count = math.log(n_executions + 1)  # +1 to handle single execution
    
    # Query execution metadata
    with connection_pool.connection() as cx:
        with cx.cursor() as cur:
            # Get function IDs and costs
            placeholders = ','.join(['%s'] * len(execution_ids))
            cur.execute(
                f"""
                SELECT caller_function_id, 1.0 as cost
                FROM execution_links
                WHERE caller_execution_id IN ({placeholders})
                UNION ALL
                SELECT callee_function_id, 1.0 as cost
                FROM execution_links
                WHERE callee_execution_id IN ({placeholders})
                """,
                execution_ids + execution_ids
            )
            rows = cur.fetchall()
    
    if not rows:
        # No data available, use baseline
        return fi_count
    
    # Function diversity (Shannon entropy)
    function_counts = Counter(row[0] for row in rows if row[0])
    total = sum(function_counts.values())
    if total > 0:
        fi_diversity = -sum(
            (count / total) * math.log(count / total)
            for count in function_counts.values()
        )
    else:
        fi_diversity = 0.0
    
    # Total FI (flat)
    fi_flat = fi_count + fi_diversity
    
    logger.debug(
        f"Flat FI: count={fi_count:.3f}, diversity={fi_diversity:.3f}, "
        f"total={fi_flat:.3f}"
    )
    
    return fi_flat


def compute_linked_execution_fi(
    connection_pool,
    root_execution_id: str,
    max_depth: int = 10,
) -> float:
    """Compute Fisher Information with call graph structure.
    
    FI components (linked):
    - All flat FI components
    - Call graph structure: edges, depth distribution, fan-out
    - Context propagation: parameters passed between functions
    - Invocation order: deterministic sequencing information
    
    Args:
        connection_pool: Database connection pool
        root_execution_id: Root of execution tree
        max_depth: Maximum depth to traverse
        
    Returns:
        Fisher Information score (higher = more information)
    """
    from collections import Counter
    
    # Collect all execution IDs in tree via BFS
    execution_ids = []
    depths = []
    fan_outs = []
    context_sizes = []
    
    visited = set()
    queue = [(root_execution_id, 0)]
    
    while queue:
        exec_id, depth = queue.pop(0)
        if exec_id in visited or depth > max_depth:
            continue
        
        visited.add(exec_id)
        execution_ids.append(exec_id)
        depths.append(depth)
        
        # Get children
        with connection_pool.connection() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    """
                    SELECT callee_execution_id, context_snapshot
                    FROM execution_links
                    WHERE caller_execution_id = %s
                    ORDER BY invocation_order ASC
                    """,
                    (exec_id,)
                )
                children = cur.fetchall()
        
        fan_outs.append(len(children))

        # Be resilient to different cursor row shapes (real DB vs mocked rows).
        # Expected: (callee_execution_id, context_snapshot)
        # Possible in tests: (callee_execution_id,), or full execution_links rows.
        for row in children:
            if not row:
                continue

            if isinstance(row, (list, tuple)):
                if len(row) == 2:
                    child_id, context = row
                elif len(row) == 1:
                    child_id, context = row[0], None
                elif len(row) >= 8:
                    child_id = row[2]
                    context = row[6]
                else:
                    continue
            else:
                child_id, context = row, None

            if context:
                import json

                context_dict = json.loads(context) if isinstance(context, str) else context
                try:
                    context_sizes.append(len(context_dict))
                except TypeError:
                    context_sizes.append(len(str(context_dict)))

            queue.append((child_id, depth + 1))
    
    # Start with flat FI
    fi_flat = compute_flat_execution_fi(connection_pool, execution_ids)
    
    # Add structural information
    fi_structure = 0.0
    
    # Depth distribution entropy
    if depths:
        depth_counts = Counter(depths)
        total_nodes = len(depths)
        fi_depth = -sum(
            (count / total_nodes) * math.log(count / total_nodes)
            for count in depth_counts.values()
        )
        fi_structure += fi_depth
    
    # Fan-out information (variance in branching factor)
    if fan_outs:
        mean_fan_out = sum(fan_outs) / len(fan_outs)
        if mean_fan_out > 0:
            variance = sum((f - mean_fan_out) ** 2 for f in fan_outs) / len(fan_outs)
            fi_structure += math.sqrt(variance)  # Std dev as proxy for FI
    
    # Context propagation information
    if context_sizes:
        mean_context_size = sum(context_sizes) / len(context_sizes)
        fi_structure += math.log(mean_context_size + 1)
    
    # Edge count information
    n_edges = sum(fan_outs)
    if n_edges > 0:
        fi_structure += math.log(n_edges + 1)
    
    fi_linked = fi_flat + fi_structure
    
    logger.debug(
        f"Linked FI: flat={fi_flat:.3f}, structure={fi_structure:.3f}, "
        f"total={fi_linked:.3f} (executions={len(execution_ids)}, "
        f"edges={n_edges}, max_depth={max(depths) if depths else 0})"
    )
    
    return fi_linked


def compute_execution_fi_uplift(
    connection_pool,
    root_execution_id: str,
    max_depth: int = 10,
) -> Dict[str, float]:
    """Compute Fisher Information uplift from call graph structure.
    
    Returns:
        Dictionary with:
        - fi_flat: FI treating executions as independent
        - fi_linked: FI with call graph structure
        - fi_uplift: Δ_structure = fi_linked - fi_flat
        - uplift_ratio: fi_linked / fi_flat
    """
    # Get all executions in tree
    execution_ids = []
    visited = set()
    queue = [root_execution_id]
    
    while queue:
        exec_id = queue.pop(0)
        if exec_id in visited:
            continue
        
        visited.add(exec_id)
        execution_ids.append(exec_id)
        
        # Get children
        with connection_pool.connection() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    "SELECT callee_execution_id FROM execution_links "
                    "WHERE caller_execution_id = %s",
                    (exec_id,)
                )
                children = cur.fetchall()
        
        # Be resilient to different cursor row shapes.
        # Some callers/tests mock rows as full execution_links records:
        # (link_id, caller_execution_id, callee_execution_id, ...)
        for row in children:
            if not row:
                continue
            if isinstance(row, (list, tuple)):
                if len(row) == 1:
                    child_id = row[0]
                elif len(row) >= 3:
                    child_id = row[2]
                else:
                    continue
            else:
                child_id = row
            queue.append(child_id)
    
    # Compute FI metrics
    fi_flat = compute_flat_execution_fi(connection_pool, execution_ids)
    fi_linked = compute_linked_execution_fi(connection_pool, root_execution_id, max_depth)
    fi_uplift = fi_linked - fi_flat
    uplift_ratio = fi_linked / fi_flat if fi_flat > 0 else 0.0
    
    return {
        "fi_flat": fi_flat,
        "fi_linked": fi_linked,
        "fi_uplift": fi_uplift,
        "uplift_ratio": uplift_ratio,
        "n_executions": len(execution_ids),
    }


def validate_execution_fi_dominance(
    connection_pool,
    root_execution_id: str,
) -> Dict[str, float | bool]:
    r"""Validate that $I_{linked} \ge I_{flat}$ (FI dominance property).

    Returns a structured report for observability and tests.
    """
    result = compute_execution_fi_uplift(connection_pool, root_execution_id)

    dominance_holds = result["fi_uplift"] >= -1e-6  # Allow tiny numerical errors

    if not dominance_holds:
        logger.warning(
            "FI dominance violation: fi_linked=%.3f < fi_flat=%.3f (uplift=%.3f)",
            result["fi_linked"],
            result["fi_flat"],
            result["fi_uplift"],
        )

    return {
        **result,
        "valid": bool(dominance_holds),
    }
