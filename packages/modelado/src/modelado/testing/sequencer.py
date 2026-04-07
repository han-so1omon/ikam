"""
Test Sequencing Framework for Multi-Step Workflows

This module provides a declarative DSL for defining, executing, and reporting
on multi-step test sequences. Key features:

1. **Sequence Definition**: Build workflows as a series of named steps
2. **State Tracking**: Automatically track state changes across steps
3. **Rollback Support**: Cleanup actions execute in reverse order on failure
4. **Dependency Tracking**: Steps can depend on prior steps' outputs
5. **Detailed Reporting**: Track timing, errors, and execution flow

Example:
    sequence = (TestSequence("artifact_workflow")
        .step("create", create_artifact)
        .step("decompose", decompose_artifact, depends_on="create")
        .step("store", store_fragments, depends_on="decompose")
        .step("reconstruct", reconstruct_artifact, depends_on="store")
        .cleanup("cleanup_fragments", cleanup_action)
    )
    
    result = sequence.execute(context)
    print(result.report())
"""

from __future__ import annotations

import asyncio
import time
import traceback
from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Callable, Dict, List, Optional, Set, Union

from pydantic import BaseModel, Field, ConfigDict


class StepStatus(str, Enum):
    """Execution status of a step."""
    PENDING = "pending"
    RUNNING = "running"
    SUCCESS = "success"
    FAILED = "failed"
    SKIPPED = "skipped"


@dataclass
class StepResult:
    """Result of executing a single step."""
    name: str
    status: StepStatus
    output: Any = None
    error: Optional[Exception] = None
    error_traceback: Optional[str] = None
    start_time: float = 0.0
    end_time: float = 0.0
    duration_ms: float = 0.0
    
    def to_dict(self) -> Dict[str, Any]:
        """Convert to dictionary for JSON serialization."""
        return {
            "name": self.name,
            "status": self.status.value,
            "output": str(self.output) if self.output is not None else None,
            "error": str(self.error) if self.error else None,
            "duration_ms": self.duration_ms,
        }


class SequenceContext(BaseModel):
    """Context passed through sequence execution.
    
    Tracks state and provides utilities for steps to access prior outputs.
    """
    name: str = Field(description="Sequence name")
    state: Dict[str, Any] = Field(default_factory=dict, description="Shared state across steps")
    metadata: Dict[str, Any] = Field(default_factory=dict, description="Custom metadata")
    
    def set(self, key: str, value: Any) -> None:
        """Store value in context state."""
        self.state[key] = value
    
    def get(self, key: str, default: Any = None) -> Any:
        """Retrieve value from context state."""
        return self.state.get(key, default)
    
    def get_step_output(self, step_name: str) -> Any:
        """Get output from a previous step."""
        return self.state.get(f"_step_output_{step_name}")
    
    def _store_step_output(self, step_name: str, output: Any) -> None:
        """Internal method to store step output."""
        self.state[f"_step_output_{step_name}"] = output
    
    model_config = ConfigDict(arbitrary_types_allowed=True)


@dataclass
class SequenceReport:
    """Comprehensive report of sequence execution."""
    name: str
    status: StepStatus
    total_duration_ms: float
    steps: List[StepResult] = field(default_factory=list)
    failed_step: Optional[str] = None
    error: Optional[Exception] = None
    
    def add_step(self, result: StepResult) -> None:
        """Add a step result to the report."""
        self.steps.append(result)
    
    def success_rate(self) -> float:
        """Percentage of successful steps."""
        if not self.steps:
            return 0.0
        successful = sum(1 for s in self.steps if s.status == StepStatus.SUCCESS)
        return (successful / len(self.steps)) * 100
    
    def report(self) -> str:
        """Generate human-readable report."""
        display_name = self.name.replace("_", " ").capitalize()
        lines = [
            f"{'='*70}",
            f"Sequence Report: {display_name}",
            f"{'='*70}",
            f"Status: {self.status.value.upper()}",
            f"Total Duration: {self.total_duration_ms:.2f}ms",
            f"Success Rate: {self.success_rate():.1f}%",
            f"",
            f"Steps ({len(self.steps)}):",
            f"-" * 70,
        ]
        
        for i, step in enumerate(self.steps, 1):
            status_icon = {
                StepStatus.SUCCESS: "✅",
                StepStatus.FAILED: "❌",
                StepStatus.SKIPPED: "⊘",
                StepStatus.RUNNING: "⏳",
                StepStatus.PENDING: "○",
            }.get(step.status, "?")
            
            lines.append(
                f"{i}. {status_icon} {step.name:<30} "
                f"{step.status.value:<10} {step.duration_ms:>7.2f}ms"
            )
            
            if step.error:
                lines.append(f"   Error: {str(step.error)}")
            
            if self.failed_step == step.name:
                lines.append(f"   ⚠️  Sequence stopped here")
        
        lines.extend([
            f"-" * 70,
            f"{'='*70}",
        ])
        
        return "\n".join(lines)
    
    def to_dict(self) -> Dict[str, Any]:
        """Convert to dictionary for JSON serialization."""
        return {
            "name": self.name,
            "status": self.status.value,
            "total_duration_ms": self.total_duration_ms,
            "success_rate": self.success_rate(),
            "failed_step": self.failed_step,
            "steps": [s.to_dict() for s in self.steps],
        }


class TestSequence:
    """Builder for defining and executing multi-step test sequences."""
    
    def __init__(self, name: str):
        """Initialize a new test sequence.
        
        Args:
            name: Human-readable sequence name
        """
        self.name = name
        self._steps: List[tuple[str, Callable, Optional[str]]] = []
        self._cleanup_steps: List[tuple[str, Callable]] = []
        self._context: Optional[SequenceContext] = None
        self._is_async = False
    
    def step(
        self,
        name: str,
        func: Callable,
        depends_on: Optional[str] = None,
    ) -> TestSequence:
        """Add a step to the sequence.
        
        Args:
            name: Unique step name
            func: Callable that executes the step. Receives (context) and returns output.
            depends_on: Optional name of prior step this depends on
        
        Returns:
            Self for chaining
        """
        self._steps.append((name, func, depends_on))
        self._is_async = self._is_async or asyncio.iscoroutinefunction(func)
        return self
    
    def cleanup(self, name: str, func: Callable) -> TestSequence:
        """Add a cleanup action (executes in reverse order on failure).
        
        Args:
            name: Cleanup action name
            func: Callable that executes cleanup. Receives (context).
        
        Returns:
            Self for chaining
        """
        self._cleanup_steps.append((name, func))
        self._is_async = self._is_async or asyncio.iscoroutinefunction(func)
        return self
    
    def execute(
        self,
        context: Optional[SequenceContext] = None,
    ) -> SequenceReport:
        """Execute the sequence synchronously.
        
        Args:
            context: Execution context (created if not provided)
        
        Returns:
            SequenceReport with detailed execution information
        """
        if self._is_async:
            # Delegate to async executor; honor existing event loop (pytest-asyncio, prod async flows)
            try:
                loop = asyncio.get_running_loop()
            except RuntimeError:
                loop = None
            if loop and loop.is_running():
                return loop.create_task(self.execute_async(context))
            return asyncio.run(self.execute_async(context))
        
        if context is None:
            context = SequenceContext(name=self.name)
        
        self._context = context
        report = SequenceReport(name=self.name, status=StepStatus.PENDING, total_duration_ms=0.0)
        start_time = time.time()
        executed_steps: List[str] = []
        
        failure = False
        failed_step_name: Optional[str] = None
        try:
            for step_name, step_func, depends_on in self._steps:
                # Check dependency
                if depends_on and depends_on not in executed_steps:
                    result = StepResult(
                        name=step_name,
                        status=StepStatus.SKIPPED,
                        error_traceback=f"Dependency '{depends_on}' not completed",
                    )
                    report.add_step(result)
                    continue
                
                # Execute step
                step_result = self._execute_step(step_name, step_func, context)
                report.add_step(step_result)
                
                if step_result.status == StepStatus.SUCCESS:
                    executed_steps.append(step_name)
                    context._store_step_output(step_name, step_result.output)
                else:
                    # Stop on failure
                    report.failed_step = step_name
                    report.status = StepStatus.FAILED
                    report.error = step_result.error
                    failure = True
                    failed_step_name = step_name
                    break
        
        except Exception as e:
            report.failed_step = "unknown"
            report.status = StepStatus.FAILED
            report.error = e
            failure = True

        # Mark remaining steps as skipped when a failure stops execution
        if failure:
            skipped_reason = f"Dependency '{failed_step_name or 'unknown'}' not completed"
            for step_name, _, _ in self._steps[len(report.steps):]:
                report.add_step(
                    StepResult(
                        name=step_name,
                        status=StepStatus.SKIPPED,
                        error_traceback=skipped_reason,
                    )
                )
        
        # Always run cleanup
        self._execute_cleanup(context, report)
        
        # Final status resolution
        if not failure and report.status != StepStatus.FAILED:
            report.status = StepStatus.SUCCESS
        
        report.total_duration_ms = (time.time() - start_time) * 1000
        return report
    
    async def execute_async(
        self,
        context: Optional[SequenceContext] = None,
    ) -> SequenceReport:
        """Execute the sequence asynchronously.
        
        Args:
            context: Execution context (created if not provided)
        
        Returns:
            SequenceReport with detailed execution information
        """
        if context is None:
            context = SequenceContext(name=self.name)
        
        self._context = context
        report = SequenceReport(name=self.name, status=StepStatus.PENDING, total_duration_ms=0.0)
        start_time = time.time()
        executed_steps: List[str] = []
        
        failure = False
        failed_step_name: Optional[str] = None
        try:
            for step_name, step_func, depends_on in self._steps:
                # Check dependency
                if depends_on and depends_on not in executed_steps:
                    result = StepResult(
                        name=step_name,
                        status=StepStatus.SKIPPED,
                        error_traceback=f"Dependency '{depends_on}' not completed",
                    )
                    report.add_step(result)
                    continue
                
                # Execute step
                step_result = await self._execute_step_async(step_name, step_func, context)
                report.add_step(step_result)
                
                if step_result.status == StepStatus.SUCCESS:
                    executed_steps.append(step_name)
                    context._store_step_output(step_name, step_result.output)
                else:
                    # Stop on failure
                    report.failed_step = step_name
                    report.status = StepStatus.FAILED
                    report.error = step_result.error
                    failure = True
                    failed_step_name = step_name
                    break
        
        except Exception as e:
            report.failed_step = "unknown"
            report.status = StepStatus.FAILED
            report.error = e
            failure = True

        # Mark remaining steps as skipped when a failure stops execution
        if failure:
            skipped_reason = f"Dependency '{failed_step_name or 'unknown'}' not completed"
            for step_name, _, _ in self._steps[len(report.steps):]:
                report.add_step(
                    StepResult(
                        name=step_name,
                        status=StepStatus.SKIPPED,
                        error_traceback=skipped_reason,
                    )
                )
        
        # Always run cleanup
        await self._execute_cleanup_async(context, report)
        
        # Final status resolution
        if not failure and report.status != StepStatus.FAILED:
            report.status = StepStatus.SUCCESS
        
        report.total_duration_ms = (time.time() - start_time) * 1000
        return report
    
    def _execute_step(
        self,
        step_name: str,
        step_func: Callable,
        context: SequenceContext,
    ) -> StepResult:
        """Execute a single step synchronously."""
        result = StepResult(name=step_name, status=StepStatus.RUNNING)
        result.start_time = time.time()
        
        try:
            result.output = step_func(context)
            result.status = StepStatus.SUCCESS
        except Exception as e:
            result.status = StepStatus.FAILED
            result.error = e
            result.error_traceback = traceback.format_exc()
        
        finally:
            result.end_time = time.time()
            result.duration_ms = (result.end_time - result.start_time) * 1000
        
        return result
    
    async def _execute_step_async(
        self,
        step_name: str,
        step_func: Callable,
        context: SequenceContext,
    ) -> StepResult:
        """Execute a single step asynchronously."""
        result = StepResult(name=step_name, status=StepStatus.RUNNING)
        result.start_time = time.time()
        
        try:
            if asyncio.iscoroutinefunction(step_func):
                result.output = await step_func(context)
            else:
                result.output = step_func(context)
            result.status = StepStatus.SUCCESS
        except Exception as e:
            result.status = StepStatus.FAILED
            result.error = e
            result.error_traceback = traceback.format_exc()
        
        finally:
            result.end_time = time.time()
            result.duration_ms = (result.end_time - result.start_time) * 1000
        
        return result
    
    def _execute_cleanup(self, context: SequenceContext, report: SequenceReport) -> None:
        """Execute cleanup steps in reverse order."""
        for cleanup_name, cleanup_func in reversed(self._cleanup_steps):
            try:
                cleanup_func(context)
            except Exception as e:
                # Log cleanup errors but don't stop
                report.add_step(StepResult(
                    name=f"cleanup_{cleanup_name}",
                    status=StepStatus.FAILED,
                    error=e,
                    error_traceback=traceback.format_exc(),
                ))
    
    async def _execute_cleanup_async(
        self,
        context: SequenceContext,
        report: SequenceReport,
    ) -> None:
        """Execute cleanup steps asynchronously in reverse order."""
        for cleanup_name, cleanup_func in reversed(self._cleanup_steps):
            try:
                if asyncio.iscoroutinefunction(cleanup_func):
                    await cleanup_func(context)
                else:
                    cleanup_func(context)
            except Exception as e:
                # Log cleanup errors but don't stop
                report.add_step(StepResult(
                    name=f"cleanup_{cleanup_name}",
                    status=StepStatus.FAILED,
                    error=e,
                    error_traceback=traceback.format_exc(),
                ))
