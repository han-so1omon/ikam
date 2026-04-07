"""Integration tests for Phase 9.5: Provenance + Fisher Information pipeline.

Tests validate complete end-to-end workflows:
1. Generate function → record provenance → calculate FI → validate dominance
2. Execute function → record execution → update FI → verify monotonicity
3. Derive function → record derivation → aggregate FI → validate composition
"""

import pytest
from typing import Dict, Any, List

from modelado.core.provenance_recorder import ProvenanceRecorder
from modelado.core.fisher_information import FisherInformationCalculator
from modelado.core.execution_provenance import (
    ExecutionProvenanceTracker,
    ExecutionResult,
)
from modelado.core.function_storage_service import FunctionStorageService
from modelado.core.function_storage import GeneratedFunctionMetadata


@pytest.mark.asyncio
class TestProvenanceE2EPipeline:
    """End-to-end tests for provenance recording pipeline."""
    
    @pytest.fixture
    def provenance_recorder(self):
        """Provenance recorder."""
        return ProvenanceRecorder()
    
    @pytest.fixture
    def calculator(self):
        """Fisher Information calculator."""
        return FisherInformationCalculator()
    
    @pytest.fixture
    async def storage_service(self):
        """In-memory function storage service."""
        service = FunctionStorageService(connection_string=None)  # In-memory
        yield service
    
    async def test_complete_generation_execution_flow(
        self,
        storage_service: FunctionStorageService,
        provenance_recorder: ProvenanceRecorder,
        calculator: FisherInformationCalculator,
    ):
        """Test: generate → store → execute → calculate FI."""
        
        # 1. Generate function
        code = """def calculate_mrr(monthly_users: int, arpu: float) -> float:
    '''Calculate Monthly Recurring Revenue.'''
    return monthly_users * arpu"""
        
        metadata = GeneratedFunctionMetadata(
            function_id="gfn_mrr_001",
            user_intent="Calculate monthly recurring revenue",
            semantic_intent="Multiply active users by average revenue per user",
            confidence=0.92,
            strategy="direct_formula",
            generator_version="1.0.0",
            context={"model": "gpt-4o-mini", "temperature": 0.2},
            semantic_reasoning="MRR is product of user count and ARPU",
            extracted_parameters=["monthly_users", "arpu"],
            constraints_enforced=["non_negative_revenue"],
        )
        
        # 2. Store function with provenance
        record = await storage_service.store_function(code, metadata)
        
        gen_event = provenance_recorder.record_generation(
            function_id=record.function_id,
            content_hash=record.content_hash,
            user_intent=metadata.user_intent,
            semantic_intent=metadata.semantic_intent,
            confidence=metadata.confidence,
            strategy=metadata.strategy,
            generator_version=metadata.generator_version,
            llm_params=metadata.context,
            semantic_reasoning=metadata.semantic_reasoning,
            extracted_parameters=metadata.extracted_parameters,
            constraints_enforced=metadata.constraints_enforced,
        )
        
        # 3. Execute function
        tracker = ExecutionProvenanceTracker(storage_service, provenance_recorder)
        
        result = await tracker.execute_with_provenance(
            function_id=record.function_id,
            inputs={"monthly_users": 5000, "arpu": 49.99},
            artifact_id="art_forecast_001",
            user_id="user_analyst_001",
        )
        
        assert result.success is True
        assert result.outputs["result"] == pytest.approx(5000 * 49.99)
        
        # 4. Calculate Fisher Information
        chain = provenance_recorder.get_derivation_chain(record.function_id)
        
        fi = calculator.calculate_function_information(
            function_id=record.function_id,
            content_hash=record.content_hash,
            code=code,
            generation_metadata=gen_event.model_dump(),
            execution_history=[e.model_dump() for e in chain.execution_events],
            derivation_info={},
        )
        
        # 5. Validate FI components
        assert fi.content_information > 0
        assert fi.generation_information > 0
        assert fi.execution_information > 0
        assert fi.total_information > fi.content_information  # Provenance adds info
    
    async def test_derivation_chain_propagates_fi(
        self,
        storage_service: FunctionStorageService,
        provenance_recorder: ProvenanceRecorder,
        calculator: FisherInformationCalculator,
    ):
        """Test: source artifact → derived function → aggregate FI."""
        
        # 1. Create source artifact (spreadsheet)
        source_artifact_id = "art_spreadsheet_revenue_001"
        
        # 2. Generate derived function
        function_id = "gfn_derived_revenue"
        code = "def revenue(units, price): return units * price"
        
        metadata = GeneratedFunctionMetadata(
            function_id=function_id,
            user_intent="Extract revenue formula",
            semantic_intent="Revenue calculation from spreadsheet",
            confidence=0.88,
            strategy="extraction",
            generator_version="1.0.0",
        )
        
        record = await storage_service.store_function(code, metadata)
        
        gen_event = provenance_recorder.record_generation(
            function_id=function_id,
            content_hash=record.content_hash,
            user_intent=metadata.user_intent,
            semantic_intent=metadata.semantic_intent,
            confidence=metadata.confidence,
            strategy=metadata.strategy,
            generator_version=metadata.generator_version,
        )
        
        # 3. Record derivation
        deriv_event = provenance_recorder.record_derivation(
            source_id=source_artifact_id,
            target_id=function_id,
            derivation_type="extraction",
            transformation_description="Extracted from cell formula B5",
            derivation_strength=0.9,
        )
        
        # 4. Calculate FI with derivation
        fi = calculator.calculate_function_information(
            function_id=function_id,
            content_hash=record.content_hash,
            code=code,
            generation_metadata=gen_event.model_dump(),
            execution_history=[],
            derivation_info={"derivations": [deriv_event.model_dump()]},
        )
        
        # 5. Validate derivation contributes to FI
        assert fi.derivation_information > 0
        assert deriv_event.derivation_id in [
            src.source_id for src in fi.information_sources
            if hasattr(src, 'source_id')
        ] or fi.derivation_information > 0  # Derivation tracked
    
    async def test_execution_history_monotonic_fi_growth(
        self,
        storage_service: FunctionStorageService,
        provenance_recorder: ProvenanceRecorder,
        calculator: FisherInformationCalculator,
    ):
        """Test: execution history grows → FI increases monotonically."""
        
        # 1. Create function
        function_id = "gfn_ltv_calc"
        code = "def ltv(arpu, churn): return arpu / churn if churn > 0 else 0"
        
        metadata = GeneratedFunctionMetadata(
            function_id=function_id,
            user_intent="Calculate customer lifetime value",
            semantic_intent="Divide ARPU by churn rate",
            confidence=0.85,
            strategy="direct_formula",
            generator_version="1.0.0",
        )
        
        record = await storage_service.store_function(code, metadata)
        
        gen_event = provenance_recorder.record_generation(
            function_id=function_id,
            content_hash=record.content_hash,
            user_intent=metadata.user_intent,
            semantic_intent=metadata.semantic_intent,
            confidence=metadata.confidence,
            strategy=metadata.strategy,
            generator_version=metadata.generator_version,
        )
        
        tracker = ExecutionProvenanceTracker(storage_service, provenance_recorder)
        
        # 2. Execute N times, measure FI growth
        fi_history = []
        
        for i in range(5):
            # Execute
            await tracker.execute_with_provenance(
                function_id=function_id,
                inputs={"arpu": 50.0 + (i * 5), "churn": 0.05 + (i * 0.01)},
            )
            
            # Calculate FI
            chain = provenance_recorder.get_derivation_chain(function_id)
            fi = calculator.calculate_function_information(
                function_id=function_id,
                content_hash=record.content_hash,
                code=code,
                generation_metadata=gen_event.model_dump(),
                execution_history=[e.model_dump() for e in chain.execution_events],
                derivation_info={},
            )
            
            fi_history.append(fi.total_information)
        
        # 3. Validate monotonic growth
        for i in range(1, len(fi_history)):
            assert fi_history[i] >= fi_history[i-1], (
                f"FI must grow monotonically: "
                f"FI[{i}]={fi_history[i]:.4f} < FI[{i-1}]={fi_history[i-1]:.4f}"
            )
    
    async def test_provenance_chain_query_completeness(
        self,
        storage_service: FunctionStorageService,
        provenance_recorder: ProvenanceRecorder,
    ):
        """Test: provenance chain includes all event types."""
        
        # 1. Create function with complete provenance
        function_id = "gfn_complete_prov"
        code = "def cac(marketing_spend, new_customers): return marketing_spend / new_customers"
        
        metadata = GeneratedFunctionMetadata(
            function_id=function_id,
            user_intent="Calculate customer acquisition cost",
            semantic_intent="Divide marketing spend by new customer count",
            confidence=0.9,
            strategy="direct_formula",
            generator_version="1.0.0",
        )
        
        record = await storage_service.store_function(code, metadata)
        
        # 2. Record generation
        provenance_recorder.record_generation(
            function_id=function_id,
            content_hash=record.content_hash,
            user_intent=metadata.user_intent,
            semantic_intent=metadata.semantic_intent,
            confidence=metadata.confidence,
            strategy=metadata.strategy,
            generator_version=metadata.generator_version,
        )
        
        # 3. Record executions
        tracker = ExecutionProvenanceTracker(storage_service, provenance_recorder)
        
        await tracker.execute_with_provenance(
            function_id=function_id,
            inputs={"marketing_spend": 100000, "new_customers": 500},
        )
        
        await tracker.execute_with_provenance(
            function_id=function_id,
            inputs={"marketing_spend": 150000, "new_customers": 800},
        )
        
        # 4. Record derivation
        provenance_recorder.record_derivation(
            source_id="art_marketing_model",
            target_id=function_id,
            derivation_type="extraction",
            transformation_description="Extracted CAC formula",
            derivation_strength=0.85,
        )
        
        # 5. Query provenance chain
        chain = provenance_recorder.get_derivation_chain(function_id)
        
        # 6. Validate completeness
        assert chain.generation_event is not None
        assert len(chain.execution_events) == 2
        assert len(chain.derivation_events) == 1
        
        # 7. Validate statistics
        stats = chain.statistics
        assert stats["generation_count"] == 1
        assert stats["execution_count"] == 2
        assert stats["derivation_count"] == 1
        assert stats["avg_execution_time_ms"] > 0
    
    async def test_batch_execution_aggregates_provenance(
        self,
        storage_service: FunctionStorageService,
        provenance_recorder: ProvenanceRecorder,
        calculator: FisherInformationCalculator,
    ):
        """Test: batch execution → aggregate provenance → combined FI."""
        
        # 1. Create multiple functions
        functions = []
        
        for i in range(3):
            function_id = f"gfn_batch_{i:03d}"
            code = f"def metric_{i}(x): return x * {i+1}"
            
            metadata = GeneratedFunctionMetadata(
                function_id=function_id,
                user_intent=f"Metric {i} calculation",
                semantic_intent=f"Multiply by {i+1}",
                confidence=0.8,
                strategy="direct",
                generator_version="1.0.0",
            )
            
            record = await storage_service.store_function(code, metadata)
            
            provenance_recorder.record_generation(
                function_id=function_id,
                content_hash=record.content_hash,
                user_intent=metadata.user_intent,
                semantic_intent=metadata.semantic_intent,
                confidence=metadata.confidence,
                strategy=metadata.strategy,
                generator_version=metadata.generator_version,
            )
            
            functions.append((function_id, code, record.content_hash))
        
        # 2. Batch execute
        tracker = ExecutionProvenanceTracker(storage_service, provenance_recorder)
        
        executions = [(fid, {"x": 100}) for fid, _, _ in functions]
        results = await tracker.execute_batch(executions)
        
        assert len(results) == 3
        assert all(r.success for r in results)
        
        # 3. Calculate aggregate FI
        fi_components = []
        
        for function_id, code, content_hash in functions:
            chain = provenance_recorder.get_derivation_chain(function_id)
            
            fi = calculator.calculate_function_information(
                function_id=function_id,
                content_hash=content_hash,
                code=code,
                generation_metadata=chain.generation_event.model_dump(),
                execution_history=[e.model_dump() for e in chain.execution_events],
                derivation_info={},
            )
            
            fi_components.append(fi)
        
        aggregate_fi = calculator.calculate_aggregate_information(fi_components)
        
        # 4. Validate aggregate
        expected_total = sum(f.total_information for f in fi_components)
        assert abs(aggregate_fi.total_information - expected_total) < 1e-6
    
    async def test_error_execution_records_provenance(
        self,
        storage_service: FunctionStorageService,
        provenance_recorder: ProvenanceRecorder,
    ):
        """Test: failed execution → error recorded in provenance."""
        
        # 1. Create function that can fail
        function_id = "gfn_division"
        code = "def safe_divide(a, b): return a / b"  # Will fail on b=0
        
        metadata = GeneratedFunctionMetadata(
            function_id=function_id,
            user_intent="Safe division",
            semantic_intent="Divide a by b",
            confidence=0.7,
            strategy="direct",
            generator_version="1.0.0",
        )
        
        record = await storage_service.store_function(code, metadata)
        
        provenance_recorder.record_generation(
            function_id=function_id,
            content_hash=record.content_hash,
            user_intent=metadata.user_intent,
            semantic_intent=metadata.semantic_intent,
            confidence=metadata.confidence,
            strategy=metadata.strategy,
            generator_version=metadata.generator_version,
        )
        
        # 2. Execute with error (divide by zero)
        tracker = ExecutionProvenanceTracker(storage_service, provenance_recorder)
        
        result = await tracker.execute_with_provenance(
            function_id=function_id,
            inputs={"a": 10, "b": 0},  # Will raise ZeroDivisionError
        )
        
        # 3. Validate error captured
        assert result.success is False
        assert result.error is not None
        assert "ZeroDivisionError" in result.error
        
        # 4. Validate provenance recorded
        chain = provenance_recorder.get_derivation_chain(function_id)
        assert len(chain.execution_events) == 1
        assert chain.execution_events[0].error is not None
        assert "ZeroDivisionError" in chain.execution_events[0].error
    
    async def test_execution_statistics_accuracy(
        self,
        storage_service: FunctionStorageService,
        provenance_recorder: ProvenanceRecorder,
    ):
        """Test: execution statistics (count, avg time, success rate)."""
        
        # 1. Create function
        function_id = "gfn_stats_test"
        code = "def compute(x): return x * 2 if x > 0 else 1 / x"  # Fails on x=0
        
        metadata = GeneratedFunctionMetadata(
            function_id=function_id,
            user_intent="Computation",
            semantic_intent="Compute result",
            confidence=0.75,
            strategy="direct",
            generator_version="1.0.0",
        )
        
        record = await storage_service.store_function(code, metadata)
        
        provenance_recorder.record_generation(
            function_id=function_id,
            content_hash=record.content_hash,
            user_intent=metadata.user_intent,
            semantic_intent=metadata.semantic_intent,
            confidence=metadata.confidence,
            strategy=metadata.strategy,
            generator_version=metadata.generator_version,
        )
        
        # 2. Execute multiple times (mix of success and failure)
        tracker = ExecutionProvenanceTracker(storage_service, provenance_recorder)
        
        inputs_list = [
            {"x": 5},    # Success
            {"x": 10},   # Success
            {"x": 0},    # Failure (ZeroDivisionError)
            {"x": 15},   # Success
            {"x": 0},    # Failure
        ]
        
        for inputs in inputs_list:
            await tracker.execute_with_provenance(function_id=function_id, inputs=inputs)
        
        # 3. Get statistics
        stats = tracker.get_execution_statistics(function_id)
        
        # 4. Validate
        assert stats["total_executions"] == 5
        assert stats["successful_executions"] == 3
        assert stats["failed_executions"] == 2
        assert stats["success_rate"] == pytest.approx(0.6)  # 3/5 = 0.6
        assert stats["avg_execution_time_ms"] > 0
