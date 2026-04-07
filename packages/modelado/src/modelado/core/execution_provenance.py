"""Execution provenance tracking for generated functions.

Integrates FunctionStorageService with ProvenanceRecorder to track complete
execution history with timing, inputs, outputs, and error handling.

Usage:
    tracker = ExecutionProvenanceTracker(storage_service, provenance_recorder)
    
    # Execute function with automatic provenance recording
    result = await tracker.execute_with_provenance(
        function_id="gfn_abc123",
        inputs={"revenue": 1000000, "price": 50.0},
        artifact_id="art_spreadsheet_123",
        user_id="user_456",
    )
    
    # Query execution history
    history = tracker.get_execution_history(function_id="gfn_abc123")
"""

from __future__ import annotations

import logging
import time
from dataclasses import dataclass
from typing import Any, Callable, Dict, List, Optional

from .function_storage import GeneratedFunctionRecord
from .function_storage_service import FunctionStorageService
from .provenance_recorder import (
    ProvenanceRecorder,
    ExecutionProvenanceEvent,
    ProvenanceChain,
)

logger = logging.getLogger(__name__)


@dataclass
class ExecutionResult:
    """Result of function execution with provenance."""
    function_id: str
    execution_id: str
    outputs: Dict[str, Any]
    execution_time_ms: float
    success: bool
    error: Optional[str] = None
    provenance_event: Optional[ExecutionProvenanceEvent] = None


class ExecutionProvenanceTracker:
    """Tracks execution provenance for generated functions.
    
    Integrates storage service with provenance recorder to provide:
    - Automatic execution timing
    - Input/output recording
    - Error handling and logging
    - Execution history queries
    - Fisher Information calculation support
    """
    
    def __init__(
        self,
        storage_service: FunctionStorageService,
        provenance_recorder: ProvenanceRecorder,
    ):
        """Initialize execution provenance tracker.
        
        Args:
            storage_service: Function storage service
            provenance_recorder: Provenance recorder
        """
        self.storage = storage_service
        self.provenance = provenance_recorder
        
        logger.info("ExecutionProvenanceTracker initialized")
    
    async def execute_with_provenance(
        self,
        function_id: str,
        inputs: Dict[str, Any],
        artifact_id: Optional[str] = None,
        user_id: Optional[str] = None,
        execution_path: Optional[str] = None,
    ) -> ExecutionResult:
        """Execute function with automatic provenance recording.
        
        Args:
            function_id: Function ID to execute
            inputs: Function input parameters
            artifact_id: Artifact ID if applied to artifact (optional)
            user_id: User who triggered execution (optional)
            execution_path: Execution trace for debugging (optional)
            
        Returns:
            ExecutionResult with outputs and provenance
        """
        # Retrieve function
        record = await self.storage.get_by_function_id(function_id)
        
        if record is None:
            error_msg = f"Function {function_id} not found in storage"
            logger.error(error_msg)
            return ExecutionResult(
                function_id=function_id,
                execution_id="",
                outputs={},
                execution_time_ms=0.0,
                success=False,
                error=error_msg,
            )
        
        # Execute with timing
        start_time = time.perf_counter()
        
        try:
            # Execute canonical code
            outputs = await self._execute_function(record, inputs)
            
            end_time = time.perf_counter()
            execution_time_ms = (end_time - start_time) * 1000
            
            # Record execution provenance
            prov_event = self.provenance.record_execution(
                function_id=function_id,
                inputs=inputs,
                outputs=outputs,
                execution_time_ms=execution_time_ms,
                execution_path=execution_path,
                error=None,
                artifact_id=artifact_id,
                user_id=user_id,
            )
            
            logger.info(
                f"Executed {function_id} successfully "
                f"({execution_time_ms:.2f}ms, {len(outputs)} outputs)"
            )
            
            return ExecutionResult(
                function_id=function_id,
                execution_id=prov_event.execution_id,
                outputs=outputs,
                execution_time_ms=execution_time_ms,
                success=True,
                provenance_event=prov_event,
            )
        
        except Exception as e:
            end_time = time.perf_counter()
            execution_time_ms = (end_time - start_time) * 1000
            
            error_msg = f"{type(e).__name__}: {str(e)}"
            logger.error(f"Execution failed for {function_id}: {error_msg}")
            
            # Record failed execution
            prov_event = self.provenance.record_execution(
                function_id=function_id,
                inputs=inputs,
                outputs={},
                execution_time_ms=execution_time_ms,
                execution_path=execution_path,
                error=error_msg,
                artifact_id=artifact_id,
                user_id=user_id,
            )
            
            return ExecutionResult(
                function_id=function_id,
                execution_id=prov_event.execution_id,
                outputs={},
                execution_time_ms=execution_time_ms,
                success=False,
                error=error_msg,
                provenance_event=prov_event,
            )
    
    async def execute_batch(
        self,
        executions: List[tuple[str, Dict[str, Any]]],
        artifact_id: Optional[str] = None,
        user_id: Optional[str] = None,
    ) -> List[ExecutionResult]:
        """Execute multiple functions in batch with provenance.
        
        Args:
            executions: List of (function_id, inputs) tuples
            artifact_id: Artifact ID if applied to artifact (optional)
            user_id: User who triggered executions (optional)
            
        Returns:
            List of ExecutionResult objects
        """
        results = []
        
        for function_id, inputs in executions:
            result = await self.execute_with_provenance(
                function_id=function_id,
                inputs=inputs,
                artifact_id=artifact_id,
                user_id=user_id,
            )
            results.append(result)
        
        logger.info(
            f"Batch execution complete: {len(results)} functions, "
            f"{sum(1 for r in results if r.success)} successful"
        )
        
        return results
    
    def get_execution_history(
        self,
        function_id: str,
        limit: Optional[int] = None,
    ) -> List[ExecutionProvenanceEvent]:
        """Get execution history for a function.
        
        Args:
            function_id: Function ID to query
            limit: Maximum events to return (optional)
            
        Returns:
            List of ExecutionProvenanceEvent objects (chronological order)
        """
        events = self.provenance.get_execution_events(function_id)
        
        if limit is not None:
            events = events[:limit]
        
        return events
    
    def get_execution_statistics(
        self,
        function_id: str,
    ) -> Dict[str, Any]:
        """Get execution statistics for a function.
        
        Args:
            function_id: Function ID to query
            
        Returns:
            Dictionary with execution stats (count, avg time, success rate)
        """
        events = self.provenance.get_execution_events(function_id)
        
        if not events:
            return {
                "execution_count": 0,
                "avg_execution_time_ms": 0.0,
                "success_rate": 0.0,
                "total_executions": 0,
                "successful_executions": 0,
                "failed_executions": 0,
            }
        
        total_count = len(events)
        successful_count = sum(1 for e in events if e.error is None)
        failed_count = total_count - successful_count
        
        avg_time = sum(e.execution_time_ms for e in events) / total_count
        success_rate = successful_count / total_count
        
        return {
            "execution_count": total_count,
            "avg_execution_time_ms": round(avg_time, 2),
            "success_rate": round(success_rate, 4),
            "total_executions": total_count,
            "successful_executions": successful_count,
            "failed_executions": failed_count,
        }
    
    def get_provenance_chain(self, function_id: str) -> ProvenanceChain:
        """Get complete provenance chain for a function.
        
        Args:
            function_id: Function ID to query
            
        Returns:
            ProvenanceChain with all events
        """
        return self.provenance.get_derivation_chain(function_id)
    
    # Private methods
    
    async def _execute_function(
        self,
        record: GeneratedFunctionRecord,
        inputs: Dict[str, Any],
    ) -> Dict[str, Any]:
        """Execute function code with inputs.
        
        Args:
            record: Function storage record
            inputs: Function input parameters
            
        Returns:
            Dictionary of output values
            
        Raises:
            Exception: If execution fails
        """
        # Compile canonical code
        code_globals = {}
        exec(record.canonical_code, code_globals)
        
        # Find function in compiled code
        # Assume function name is first def in code
        func_name = None
        for name, obj in code_globals.items():
            if callable(obj) and not name.startswith('__'):
                func_name = name
                break
        
        if func_name is None:
            raise ValueError("No callable function found in code")
        
        func = code_globals[func_name]
        
        # Execute function
        result = func(**inputs)
        
        # Return as dictionary
        if isinstance(result, dict):
            return result
        else:
            return {"result": result}


# Integration with FunctionStorageService

class ProvenanceEnabledStorageService(FunctionStorageService):
    """FunctionStorageService with integrated provenance recording.
    
    Extends FunctionStorageService to automatically record generation provenance
    when storing functions.
    """
    
    def __init__(
        self,
        connection_string: Optional[str] = None,
        use_blake3: bool = True,
        provenance_recorder: Optional[ProvenanceRecorder] = None,
    ):
        """Initialize provenance-enabled storage service.
        
        Args:
            connection_string: PostgreSQL connection string (optional)
            use_blake3: Use BLAKE3 for hashing (faster)
            provenance_recorder: Provenance recorder (optional, creates new if None)
        """
        super().__init__(connection_string, use_blake3)
        
        self.provenance = provenance_recorder or ProvenanceRecorder()
        
        logger.info("ProvenanceEnabledStorageService initialized")
    
    async def store_function(
        self,
        code: str,
        metadata: Any,  # GeneratedFunctionMetadata
        cache_key: Optional[str] = None,
        record_provenance: bool = True,
    ) -> Any:  # GeneratedFunctionRecord
        """Store function with automatic provenance recording.
        
        Args:
            code: Generated Python function code
            metadata: Generation metadata
            cache_key: Semantic cache key (optional)
            record_provenance: Whether to record generation provenance
            
        Returns:
            GeneratedFunctionRecord with storage metadata
        """
        # Store function (parent implementation)
        record = await super().store_function(code, metadata, cache_key)
        
        # Record generation provenance
        if record_provenance and not record.deduplicated:
            # Only record provenance for first storage (not duplicates)
            self.provenance.record_generation(
                function_id=record.function_id,
                content_hash=record.content_hash,
                user_intent=metadata.user_intent,
                semantic_intent=metadata.semantic_intent,
                confidence=metadata.confidence,
                strategy=metadata.strategy,
                generator_version=metadata.generator_version,
                llm_params=metadata.context,  # Use context for LLM params
                semantic_reasoning=metadata.semantic_reasoning,
                extracted_parameters=metadata.extracted_parameters,
                constraints_enforced=metadata.constraints_enforced,
                cache_key=cache_key,
            )
            
            logger.debug(f"Recorded generation provenance for {record.function_id}")
        
        return record
