"""Information dominance validation tests (Phase 9.5).

Validates the core IKAM theorem: I_IKAM(θ) ≥ I_RAG(θ) + Δ_provenance(θ)

Tests verify that IKAM's provenance-enriched representation achieves strictly
higher Fisher Information than baseline RAG systems for all test scenarios.

Mathematical foundation from FISHER_INFORMATION_GAINS.md:
- I_{(X,Y)}(θ) = I_X(θ) + E_X[I_{Y|X}(θ)] ⪰ I_X(θ)
- Provenance delta Δ_provenance_FI(θ) := E_X[I_{Y|X}(θ)] ⪰ 0
"""

import pytest
from typing import Dict, Any, List
from dataclasses import dataclass

from modelado.core.fisher_information import (
    FisherInformationCalculator,
    InformationDominanceComparison,
    InformationSource,
)
from modelado.core.provenance_recorder import (
    ProvenanceRecorder,
    GenerationProvenanceEvent,
    ExecutionProvenanceEvent,
    DerivationProvenanceEvent,
)
from modelado.core.function_storage import GeneratedFunctionMetadata


@dataclass
class BaselineFunction:
    """Baseline function (RAG-style) without provenance."""
    function_id: str
    content_hash: str
    code: str
    metadata: Dict[str, Any]


@dataclass
class IKAMFunction:
    """IKAM function with complete provenance."""
    function_id: str
    content_hash: str
    code: str
    metadata: GeneratedFunctionMetadata
    generation_event: GenerationProvenanceEvent
    execution_events: List[ExecutionProvenanceEvent]
    derivation_events: List[DerivationProvenanceEvent]


class TestInformationDominance:
    """Tests for Fisher Information dominance (IKAM ≥ baseline)."""
    
    @pytest.fixture
    def calculator(self):
        """Fisher Information calculator."""
        return FisherInformationCalculator()
    
    @pytest.fixture
    def provenance_recorder(self):
        """Provenance recorder."""
        return ProvenanceRecorder()
    
    @pytest.fixture
    def baseline_function(self) -> BaselineFunction:
        """Baseline RAG function without provenance."""
        return BaselineFunction(
            function_id="gfn_baseline_001",
            content_hash="abc123",
            code="""def calculate_revenue(units: int, price: float) -> float:
    return units * price""",
            metadata={
                "name": "calculate_revenue",
                "created_at": "2025-01-15T10:00:00Z",
            },
        )
    
    @pytest.fixture
    def ikam_function(self, provenance_recorder) -> IKAMFunction:
        """IKAM function with complete provenance."""
        function_id = "gfn_ikam_001"
        code = """def calculate_revenue(units: int, price: float) -> float:
    \"\"\"Calculate revenue from units and price.\"\"\"
    return units * price"""
        
        # Generation provenance
        gen_event = provenance_recorder.record_generation(
            function_id=function_id,
            content_hash="def456",
            user_intent="Calculate total revenue from unit sales",
            semantic_intent="Multiply units sold by unit price",
            confidence=0.95,
            strategy="direct_formula",
            generator_version="1.0.0",
            llm_params={
                "model": "gpt-4o-mini",
                "temperature": 0.3,
                "max_tokens": 500,
            },
            semantic_reasoning="Revenue is product of quantity and price",
            extracted_parameters=["units", "price"],
            constraints_enforced=["non_negative_revenue"],
        )
        
        # Execution provenance
        exec_events = [
            provenance_recorder.record_execution(
                function_id=function_id,
                inputs={"units": 1000, "price": 50.0},
                outputs={"result": 50000.0},
                execution_time_ms=1.2,
            ),
            provenance_recorder.record_execution(
                function_id=function_id,
                inputs={"units": 500, "price": 75.0},
                outputs={"result": 37500.0},
                execution_time_ms=0.9,
            ),
        ]
        
        # Derivation provenance (derived from spreadsheet)
        deriv_events = [
            provenance_recorder.record_derivation(
                source_id="art_spreadsheet_001",
                target_id=function_id,
                derivation_type="extraction",
                transformation_description="Extracted revenue formula from cell B5",
                derivation_strength=0.9,
            ),
        ]
        
        metadata = GeneratedFunctionMetadata(
            function_id=function_id,
            user_intent="Calculate total revenue from unit sales",
            semantic_intent="Multiply units sold by unit price",
            confidence=0.95,
            strategy="direct_formula",
            generator_version="1.0.0",
            context={
                "model": "gpt-4o-mini",
                "temperature": 0.3,
            },
            semantic_reasoning="Revenue is product of quantity and price",
            extracted_parameters=["units", "price"],
            constraints_enforced=["non_negative_revenue"],
        )
        
        return IKAMFunction(
            function_id=function_id,
            content_hash="def456",
            code=code,
            metadata=metadata,
            generation_event=gen_event,
            execution_events=exec_events,
            derivation_events=deriv_events,
        )
    
    def test_basic_information_dominance(
        self,
        calculator: FisherInformationCalculator,
        baseline_function: BaselineFunction,
        ikam_function: IKAMFunction,
    ):
        """Test that IKAM achieves higher FI than baseline for same content."""
        # Calculate baseline FI (content only)
        baseline_fi = calculator.calculate_function_information(
            function_id=baseline_function.function_id,
            content_hash=baseline_function.content_hash,
            code=baseline_function.code,
            generation_metadata={},  # No provenance
            execution_history=[],
            derivation_info={},
        )
        
        # Calculate IKAM FI (content + provenance)
        ikam_fi = calculator.calculate_function_information(
            function_id=ikam_function.function_id,
            content_hash=ikam_function.content_hash,
            code=ikam_function.code,
            generation_metadata=ikam_function.generation_event.model_dump(),
            execution_history=[e.model_dump() for e in ikam_function.execution_events],
            derivation_info={
                "derivations": [d.model_dump() for d in ikam_function.derivation_events],
            },
        )
        
        # Validate dominance
        assert ikam_fi.total_information > baseline_fi.total_information, (
            f"IKAM FI ({ikam_fi.total_information:.4f}) must exceed "
            f"baseline FI ({baseline_fi.total_information:.4f})"
        )
        
        # Verify provenance delta is positive
        provenance_delta = ikam_fi.total_information - baseline_fi.total_information
        assert provenance_delta > 0, "Provenance delta must be positive"
    
    def test_information_decomposition(
        self,
        calculator: FisherInformationCalculator,
        ikam_function: IKAMFunction,
    ):
        """Test FI decomposition by source (content, generation, execution, derivation)."""
        fi_components = calculator.calculate_function_information(
            function_id=ikam_function.function_id,
            content_hash=ikam_function.content_hash,
            code=ikam_function.code,
            generation_metadata=ikam_function.generation_event.model_dump(),
            execution_history=[e.model_dump() for e in ikam_function.execution_events],
            derivation_info={
                "derivations": [d.model_dump() for d in ikam_function.derivation_events],
            },
        )
        
        # All components should be non-negative
        assert fi_components.content_information >= 0
        assert fi_components.generation_information >= 0
        assert fi_components.execution_information >= 0
        assert fi_components.derivation_information >= 0
        
        # Total should equal sum of components
        total = (
            fi_components.content_information
            + fi_components.generation_information
            + fi_components.execution_information
            + fi_components.derivation_information
        )
        assert abs(total - fi_components.total_information) < 1e-6
    
    def test_execution_history_increases_fi(
        self,
        calculator: FisherInformationCalculator,
        ikam_function: IKAMFunction,
    ):
        """Test that execution history increases Fisher Information."""
        # FI without execution history
        fi_no_exec = calculator.calculate_function_information(
            function_id=ikam_function.function_id,
            content_hash=ikam_function.content_hash,
            code=ikam_function.code,
            generation_metadata=ikam_function.generation_event.model_dump(),
            execution_history=[],  # No execution history
            derivation_info={
                "derivations": [d.model_dump() for d in ikam_function.derivation_events],
            },
        )
        
        # FI with execution history
        fi_with_exec = calculator.calculate_function_information(
            function_id=ikam_function.function_id,
            content_hash=ikam_function.content_hash,
            code=ikam_function.code,
            generation_metadata=ikam_function.generation_event.model_dump(),
            execution_history=[e.model_dump() for e in ikam_function.execution_events],
            derivation_info={
                "derivations": [d.model_dump() for d in ikam_function.derivation_events],
            },
        )
        
        # Execution history must increase FI
        assert fi_with_exec.total_information > fi_no_exec.total_information
        assert fi_with_exec.execution_information > fi_no_exec.execution_information
    
    def test_derivation_strength_affects_fi(
        self,
        calculator: FisherInformationCalculator,
        provenance_recorder: ProvenanceRecorder,
    ):
        """Test that derivation strength affects Fisher Information."""
        function_id = "gfn_test_001"
        code = "def f(x): return x * 2"
        
        # Weak derivation (strength=0.3)
        weak_deriv = provenance_recorder.record_derivation(
            source_id="art_source_001",
            target_id=function_id,
            derivation_type="inference",
            transformation_description="Inferred from pattern",
            derivation_strength=0.3,
        )
        
        fi_weak = calculator.calculate_function_information(
            function_id=function_id,
            content_hash="abc123",
            code=code,
            generation_metadata={},
            execution_history=[],
            derivation_info={"derivations": [weak_deriv.model_dump()]},
        )
        
        # Strong derivation (strength=0.9)
        strong_deriv = provenance_recorder.record_derivation(
            source_id="art_source_001",
            target_id=function_id,
            derivation_type="extraction",
            transformation_description="Direct extraction from source",
            derivation_strength=0.9,
        )
        
        fi_strong = calculator.calculate_function_information(
            function_id=function_id,
            content_hash="abc123",
            code=code,
            generation_metadata={},
            execution_history=[],
            derivation_info={"derivations": [strong_deriv.model_dump()]},
        )
        
        # Stronger derivation must yield higher FI
        assert fi_strong.derivation_information > fi_weak.derivation_information
    
    def test_compare_with_baseline_validation(
        self,
        calculator: FisherInformationCalculator,
        baseline_function: BaselineFunction,
        ikam_function: IKAMFunction,
    ):
        """Test compare_with_baseline() method for dominance validation."""
        # Calculate baseline FI
        baseline_fi = calculator.calculate_function_information(
            function_id=baseline_function.function_id,
            content_hash=baseline_function.content_hash,
            code=baseline_function.code,
            generation_metadata={},
            execution_history=[],
            derivation_info={},
        )
        
        # Calculate IKAM FI
        ikam_fi = calculator.calculate_function_information(
            function_id=ikam_function.function_id,
            content_hash=ikam_function.content_hash,
            code=ikam_function.code,
            generation_metadata=ikam_function.generation_event.model_dump(),
            execution_history=[e.model_dump() for e in ikam_function.execution_events],
            derivation_info={
                "derivations": [d.model_dump() for d in ikam_function.derivation_events],
            },
        )
        
        # Compare with baseline
        comparison = calculator.compare_with_baseline(ikam_fi, baseline_fi)
        
        assert comparison.ikam_information == ikam_fi.total_information
        assert comparison.baseline_information == baseline_fi.total_information
        assert comparison.provenance_delta > 0
        assert comparison.dominance_ratio > 1.0
        assert comparison.dominates_baseline is True
    
    def test_aggregate_information_dominance(
        self,
        calculator: FisherInformationCalculator,
        provenance_recorder: ProvenanceRecorder,
    ):
        """Test aggregate FI calculation for multiple functions."""
        # Create 3 functions with varying provenance
        functions_fi = []
        
        for i in range(3):
            function_id = f"gfn_agg_{i:03d}"
            code = f"def f{i}(x): return x * {i+1}"
            
            gen_event = provenance_recorder.record_generation(
                function_id=function_id,
                content_hash=f"hash_{i}",
                user_intent=f"Function {i}",
                semantic_intent=f"Multiply by {i+1}",
                confidence=0.8 + (i * 0.05),
                strategy="direct",
                generator_version="1.0.0",
            )
            
            fi = calculator.calculate_function_information(
                function_id=function_id,
                content_hash=f"hash_{i}",
                code=code,
                generation_metadata=gen_event.model_dump(),
                execution_history=[],
                derivation_info={},
            )
            
            functions_fi.append(fi)
        
        # Calculate aggregate FI
        aggregate_fi = calculator.calculate_aggregate_information(functions_fi)
        
        # Aggregate FI should be sum of individual FIs
        expected_total = sum(f.total_information for f in functions_fi)
        assert abs(aggregate_fi.total_information - expected_total) < 1e-6
    
    def test_validate_information_dominance_method(
        self,
        calculator: FisherInformationCalculator,
        ikam_function: IKAMFunction,
    ):
        """Test validate_information_dominance() convenience method."""
        # This method should validate I_IKAM ≥ I_baseline + Δ_provenance
        
        ikam_fi = calculator.calculate_function_information(
            function_id=ikam_function.function_id,
            content_hash=ikam_function.content_hash,
            code=ikam_function.code,
            generation_metadata=ikam_function.generation_event.model_dump(),
            execution_history=[e.model_dump() for e in ikam_function.execution_events],
            derivation_info={
                "derivations": [d.model_dump() for d in ikam_function.derivation_events],
            },
        )
        
        # Baseline FI (content only)
        baseline_fi = calculator.calculate_function_information(
            function_id=ikam_function.function_id,
            content_hash=ikam_function.content_hash,
            code=ikam_function.code,
            generation_metadata={},
            execution_history=[],
            derivation_info={},
        )
        
        # Validate dominance
        is_valid = calculator.validate_information_dominance(ikam_fi, baseline_fi)
        assert is_valid is True
    
    def test_zero_provenance_equals_baseline(
        self,
        calculator: FisherInformationCalculator,
    ):
        """Test that IKAM with zero provenance equals baseline."""
        function_id = "gfn_zero_prov"
        code = "def f(x): return x"
        content_hash = "hash_zero"
        
        # IKAM with zero provenance
        ikam_fi = calculator.calculate_function_information(
            function_id=function_id,
            content_hash=content_hash,
            code=code,
            generation_metadata={},  # Empty provenance
            execution_history=[],
            derivation_info={},
        )
        
        # Baseline
        baseline_fi = calculator.calculate_function_information(
            function_id=function_id,
            content_hash=content_hash,
            code=code,
            generation_metadata={},
            execution_history=[],
            derivation_info={},
        )
        
        # Should be equal (within floating point tolerance)
        assert abs(ikam_fi.total_information - baseline_fi.total_information) < 1e-6
    
    def test_information_sources_attribution(
        self,
        calculator: FisherInformationCalculator,
        ikam_function: IKAMFunction,
    ):
        """Test that information sources are correctly attributed."""
        fi = calculator.calculate_function_information(
            function_id=ikam_function.function_id,
            content_hash=ikam_function.content_hash,
            code=ikam_function.code,
            generation_metadata=ikam_function.generation_event.model_dump(),
            execution_history=[e.model_dump() for e in ikam_function.execution_events],
            derivation_info={
                "derivations": [d.model_dump() for d in ikam_function.derivation_events],
            },
        )
        
        # Verify information sources are tracked
        assert InformationSource.CONTENT in fi.sources
        assert InformationSource.GENERATION_METADATA in fi.sources
        assert InformationSource.EXECUTION_HISTORY in fi.sources
        assert InformationSource.DERIVATION_STRUCTURE in fi.sources
        
        # Each source should contribute positive information
        assert fi.content_information > 0
        assert fi.generation_information > 0
        assert fi.execution_information > 0
        assert fi.derivation_information > 0
