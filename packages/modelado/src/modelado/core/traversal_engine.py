"""Step-based execution traversal engine for operation call graphs.

This module implements bounded, concurrent traversal of ExecutionLinkGraph
with realistic timing models (spike/decay), progress streaming, and full
provenance tracking.

Mathematical Foundation:
- Step-based clock: Discrete time steps enabling synchronized traversal
- Spike/Decay Model: Execution time = base_time * (1 + spike_factor * e^(-decay_rate * steps))
- Bounded Concurrency: max_workers limit prevents resource exhaustion
- Breadth-First/Depth-First: Configurable traversal order
"""

import asyncio
import logging
from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from datetime import datetime, timedelta
from enum import Enum
from typing import Optional, Dict, List, Set, Any, Callable, Coroutine
import math
import uuid

from pydantic import BaseModel, Field

logger = logging.getLogger(__name__)


class TraversalOrder(Enum):
    """Traversal strategy for execution graph."""
    BREADTH_FIRST = "breadth_first"  # Process all children before grandchildren
    DEPTH_FIRST = "depth_first"      # Process full path before siblings


# Provenance Events for Traversal


class TraversalStepEvent(BaseModel):
    """Records a single step during execution traversal.
    
    Fisher Information Contribution:
    - step_number: Sequential ordering reveals traversal strategy
    - duration_seconds: Execution time constrains performance characteristics
    - depth_level: Graph depth reveals call structure complexity
    - sibling_count: Branching factor indicates call parallelism
    - parent_execution_id: Caller-callee relationships enable derivation tracking
    
    Example:
      Step 1: exec_abc (function_1) → 0.5s
      Step 2: exec_def (function_2, parent=exec_abc) → 0.3s  [Fisher Info: traversal order + timing]
      Step 3: exec_ghi (function_3, parent=exec_abc) → 0.4s  [Fisher Info: sibling relationship]
    """
    
    event_id: str = Field(default_factory=lambda: f"step_{uuid.uuid4().hex[:16]}")
    timestamp: datetime = Field(default_factory=datetime.utcnow)
    
    # Traversal identifiers
    traversal_id: str = Field(
        ...,
        description="Unique identifier for this traversal session"
    )
    step_number: int = Field(
        ...,
        ge=0,
        description="Sequential step number (0-indexed)"
    )
    
    # Execution context
    execution_id: str = Field(
        ...,
        description="Execution being processed in this step"
    )
    function_id: str = Field(
        ...,
        description="Function ID for this execution"
    )
    parent_execution_id: Optional[str] = Field(
        None,
        description="Parent execution ID (None for root)"
    )
    
    # Traversal metrics
    duration_seconds: float = Field(
        ...,
        ge=0.0,
        description="Time spent processing this step"
    )
    depth_level: int = Field(
        ...,
        ge=0,
        description="Depth in call graph (0=root)"
    )
    sibling_count: int = Field(
        ...,
        ge=0,
        description="Number of siblings at this depth (0=only child)"
    )
    
    # Status
    status: str = Field(
        ...,
        description="Step status: pending, running, completed, failed"
    )
    error_message: Optional[str] = Field(
        None,
        description="Error message if step failed"
    )
    
    # Fisher Information
    information_content: float = Field(
        default=0.0,
        ge=0.0,
        description="Fisher Information for this step"
    )
    
    def compute_fisher_information(self) -> float:
        """Compute Fisher Information from traversal metrics.
        
        I_step(θ) = duration × log(depth + 1) / (sibling_count + 1)
        
        Intuition:
        - Longer duration = more computational evidence
        - Greater depth = more sequential constraints
        - More siblings = greater parallelism (less evidence per execution)
        
        Returns:
            Fisher Information value
        """
        if self.duration_seconds <= 0:
            return 0.0
        
        # Avoid log(0) with offset
        depth_factor = math.log(self.depth_level + 2)
        
        # More siblings → less information per execution
        sibling_factor = 1.0 / (self.sibling_count + 1.0)
        
        return self.duration_seconds * depth_factor * sibling_factor


@dataclass
class TimingModel:
    """Execution timing model parameters for realistic simulation."""
    
    base_time_seconds: float = 1.0          # Baseline execution time
    spike_factor: float = 0.5                # Spike magnitude (0-1)
    decay_rate: float = 0.3                  # Exponential decay rate
    
    def calculate_duration(self, step_number: int) -> float:
        """Calculate execution duration at given step.
        
        Formula: duration = base_time * (1 + spike_factor * e^(-decay_rate * step))
        
        Args:
            step_number: Current step in execution (0-indexed)
            
        Returns:
            Execution time in seconds
        """
        spike = self.spike_factor * math.exp(-self.decay_rate * step_number)
        return self.base_time_seconds * (1.0 + spike)
    
    def __post_init__(self):
        """Validate timing model parameters."""
        if self.base_time_seconds <= 0:
            raise ValueError("base_time_seconds must be positive")
        if not (0 <= self.spike_factor <= 1):
            raise ValueError("spike_factor must be in [0, 1]")
        if self.decay_rate < 0:
            raise ValueError("decay_rate must be non-negative")


@dataclass
class ExecutionStep:
    """Single step in execution traversal."""
    
    step_number: int                           # Sequential step counter
    execution_id: str                          # Execution being processed
    function_id: str                           # Function ID
    parent_execution_id: Optional[str]         # Parent execution (None for root)
    timestamp: datetime = field(default_factory=datetime.utcnow)
    duration_seconds: float = 0.0              # Time spent on this step
    status: str = "pending"                    # pending, running, completed, failed
    error_message: Optional[str] = None        # Error if failed
    
    def __post_init__(self):
        """Validate step parameters."""
        # Allow -1 as sentinel for "not yet assigned by traverser"
        if self.step_number < -1:
            raise ValueError("step_number must be >= -1 (use -1 for not-yet-assigned)")
        if not self.execution_id:
            raise ValueError("execution_id is required")
        if not self.function_id:
            raise ValueError("function_id is required")


@dataclass
class TraversalProgress:
    """Progress metrics for ongoing traversal."""
    
    total_steps: int = 0                       # Total steps to process
    completed_steps: int = 0                   # Steps completed
    failed_steps: int = 0                      # Steps failed
    elapsed_time_seconds: float = 0.0          # Total elapsed time
    active_workers: int = 0                    # Current concurrent workers
    last_step_number: int = -1                 # Most recent step processed
    
    @property
    def completion_percent(self) -> float:
        """Calculate completion percentage."""
        if self.total_steps == 0:
            return 0.0
        return (self.completed_steps / self.total_steps) * 100.0
    
    @property
    def success_rate_percent(self) -> float:
        """Calculate success rate percentage."""
        total_finished = self.completed_steps + self.failed_steps
        if total_finished == 0:
            return 0.0
        return (self.completed_steps / total_finished) * 100.0
    
    @property
    def average_step_time(self) -> float:
        """Calculate average time per completed step."""
        if self.completed_steps == 0:
            return 0.0
        return self.elapsed_time_seconds / self.completed_steps


@dataclass
class TraversalConfig:
    """Configuration for execution traversal."""
    
    order: TraversalOrder = TraversalOrder.BREADTH_FIRST
    max_workers: int = 4                       # Max concurrent executions
    timing_model: TimingModel = field(default_factory=TimingModel)
    timeout_seconds: Optional[float] = None    # Overall timeout
    max_depth: Optional[int] = None            # Max traversal depth
    progress_callback: Optional[Callable[[TraversalProgress], Coroutine]] = None
    
    def __post_init__(self):
        """Validate traversal config."""
        if self.max_workers < 1:
            raise ValueError("max_workers must be >= 1")
        if self.timeout_seconds is not None and self.timeout_seconds <= 0:
            raise ValueError("timeout_seconds must be positive")
        if self.max_depth is not None and self.max_depth < 0:
            raise ValueError("max_depth must be non-negative")


class ExecutionTraverser(ABC):
    """Abstract base for execution traversal implementations."""
    
    @abstractmethod
    async def traverse(
        self,
        root_execution_id: str,
        config: TraversalConfig,
    ) -> TraversalProgress:
        """Traverse execution graph starting from root.
        
        Args:
            root_execution_id: Starting execution node
            config: Traversal configuration
            
        Returns:
            Final progress metrics
        """
        pass
    
    @abstractmethod
    async def get_children(self, execution_id: str) -> List[ExecutionStep]:
        """Get child executions for given execution.
        
        Args:
            execution_id: Parent execution ID
            
        Returns:
            List of child execution steps
        """
        pass


class BreadthFirstTraverser(ExecutionTraverser):
    """Breadth-first execution traversal (queue-based)."""
    
    def __init__(
        self,
        get_children_fn: Callable[[str], Coroutine],
        event_recorder: Optional[Any] = None,  # InvocationEventRecorder instance
        traversal_id: Optional[str] = None,
    ):
        """Initialize traverser.
        
        Args:
            get_children_fn: Async function to fetch child executions
            event_recorder: Optional InvocationEventRecorder for provenance tracking
            traversal_id: Unique ID for this traversal session
        """
        self._get_children = get_children_fn
        self._step_number = 0
        self.event_recorder = event_recorder
        self.traversal_id = traversal_id or f"trav_{uuid.uuid4().hex[:16]}"
    
    async def traverse(
        self,
        root_execution_id: str,
        config: TraversalConfig,
    ) -> TraversalProgress:
        """Traverse execution graph breadth-first.
        
        Uses queue-based processing to maintain level ordering.
        """
        progress = TraversalProgress()
        start_time = datetime.utcnow()
        
        # Initialize queue with root
        queue: asyncio.Queue[ExecutionStep] = asyncio.Queue()
        await queue.put(ExecutionStep(
            step_number=0,
            execution_id=root_execution_id,
            function_id="root",
            parent_execution_id=None,
        ))
        
        # Create worker tasks
        workers = []
        for _ in range(config.max_workers):
            worker = asyncio.create_task(
                self._worker(queue, config, progress)
            )
            workers.append(worker)
        
        try:
            # Wait for queue to be empty and workers to finish
            await queue.join()
        finally:
            # Cancel workers
            for worker in workers:
                worker.cancel()
            await asyncio.gather(*workers, return_exceptions=True)
        
        # Finalize progress
        progress.elapsed_time_seconds = (datetime.utcnow() - start_time).total_seconds()
        
        if config.progress_callback:
            await config.progress_callback(progress)
        
        return progress
    
    async def _worker(
        self,
        queue: asyncio.Queue,
        config: TraversalConfig,
        progress: TraversalProgress,
    ) -> None:
        """Worker coroutine processing steps from queue."""
        # Track depth levels for Fisher Information
        depth_map: Dict[str, int] = {None: 0}  # parent_id -> depth
        sibling_counters: Dict[str, int] = {}  # parent_id -> sibling_count
        
        while True:
            try:
                # Get step with timeout
                if config.timeout_seconds:
                    step = await asyncio.wait_for(
                        queue.get(),
                        timeout=config.timeout_seconds,
                    )
                else:
                    step = await queue.get()
                
                try:
                    progress.active_workers += 1
                    
                    # Simulate execution with timing model
                    duration = config.timing_model.calculate_duration(step.step_number)
                    await asyncio.sleep(duration)
                    
                    step.status = "completed"
                    step.duration_seconds = duration
                    progress.completed_steps += 1
                    progress.last_step_number = step.step_number
                    
                    # Calculate depth for Fisher Information
                    parent_id = step.parent_execution_id
                    depth_level = depth_map.get(parent_id, 0)
                    
                    # Track sibling count for this parent
                    if parent_id not in sibling_counters:
                        sibling_counters[parent_id] = 0
                    sibling_count = sibling_counters[parent_id]
                    sibling_counters[parent_id] += 1
                    
                    # Record traversal step event if recorder available
                    if self.event_recorder:
                        step_event = TraversalStepEvent(
                            traversal_id=self.traversal_id,
                            step_number=step.step_number,
                            execution_id=step.execution_id,
                            function_id=step.function_id,
                            parent_execution_id=step.parent_execution_id,
                            duration_seconds=duration,
                            depth_level=depth_level,
                            sibling_count=sibling_count,
                            status=step.status,
                            error_message=step.error_message,
                        )
                        step_event.information_content = step_event.compute_fisher_information()
                        
                        # Record event (async, but fire-and-forget)
                        try:
                            await self.event_recorder.record_traversal_step(step_event)
                        except Exception as e:
                            logger.error(f"Failed to record traversal step event: {e}")
                    
                    # Fetch and queue children
                    try:
                        children = await self._get_children(step.execution_id)
                        for child in children:
                            # Assign step number and parent
                            if child.step_number < 0:  # Only if not already assigned
                                child.step_number = self._step_number
                                self._step_number += 1
                            if child.parent_execution_id is None:
                                child.parent_execution_id = step.execution_id
                            
                            # Update depth map for children
                            depth_map[step.execution_id] = depth_level + 1
                            
                            progress.total_steps += 1
                            await queue.put(child)
                    except Exception as e:
                        logger.error(f"Failed to get children of {step.execution_id}: {e}")
                    
                    # Notify progress
                    if config.progress_callback:
                        await config.progress_callback(progress)
                    
                except asyncio.CancelledError:
                    raise
                except Exception as e:
                    step.status = "failed"
                    step.error_message = str(e)
                    progress.failed_steps += 1
                    logger.error(f"Step execution failed: {e}")
                
                finally:
                    progress.active_workers -= 1
                    queue.task_done()
            
            except asyncio.CancelledError:
                break
            except asyncio.TimeoutError:
                logger.warning("Worker timeout waiting for queue item")
                break
    
    async def get_children(self, execution_id: str) -> List[ExecutionStep]:
        """Fetch child executions."""
        return await self._get_children(execution_id)


class DepthFirstTraverser(ExecutionTraverser):
    """Depth-first execution traversal (stack-based)."""
    
    def __init__(
        self,
        get_children_fn: Callable[[str], Coroutine],
        event_recorder: Optional[Any] = None,  # InvocationEventRecorder instance
        traversal_id: Optional[str] = None,
    ):
        """Initialize traverser.
        
        Args:
            get_children_fn: Async function to fetch child executions
            event_recorder: Optional InvocationEventRecorder for provenance tracking
            traversal_id: Unique ID for this traversal session
        """
        self._get_children = get_children_fn
        self._step_number = 0
        self.event_recorder = event_recorder
        self.traversal_id = traversal_id or f"trav_{uuid.uuid4().hex[:16]}"
        self.sibling_counters: Dict[str, int] = {}  # parent_id -> sibling_count
        self.depth_map: Dict[str, int] = {None: 0}  # parent_id -> depth
    
    async def traverse(
        self,
        root_execution_id: str,
        config: TraversalConfig,
    ) -> TraversalProgress:
        """Traverse execution graph depth-first.
        
        Uses recursion with controlled depth and worker pool.
        """
        progress = TraversalProgress()
        start_time = datetime.utcnow()
        
        # Create semaphore for worker concurrency control
        semaphore = asyncio.Semaphore(config.max_workers)
        
        # Start recursive traversal
        try:
            await self._traverse_recursive(
                root_execution_id,
                config,
                progress,
                semaphore,
                depth=0,
            )
        except asyncio.CancelledError:
            pass
        
        # Finalize progress
        progress.elapsed_time_seconds = (datetime.utcnow() - start_time).total_seconds()
        
        if config.progress_callback:
            await config.progress_callback(progress)
        
        return progress
    
    async def _traverse_recursive(
        self,
        execution_id: str,
        config: TraversalConfig,
        progress: TraversalProgress,
        semaphore: asyncio.Semaphore,
        depth: int,
        parent_execution_id: Optional[str] = None,
        step_number: Optional[int] = None,
    ) -> None:
        """Recursively traverse execution tree."""
        # Check depth limit
        if config.max_depth is not None and depth > config.max_depth:
            return
        
        # Process this node
        async with semaphore:
            try:
                # Simulate execution
                duration = config.timing_model.calculate_duration(self._step_number)
                await asyncio.sleep(duration)
                
                progress.completed_steps += 1
                progress.last_step_number = self._step_number
                current_step_number = self._step_number
                self._step_number += 1
                
                # Track sibling count for Fisher Information
                if parent_execution_id not in self.sibling_counters:
                    self.sibling_counters[parent_execution_id] = 0
                sibling_count = self.sibling_counters[parent_execution_id]
                self.sibling_counters[parent_execution_id] += 1
                
                # Get current depth
                current_depth = self.depth_map.get(parent_execution_id, 0)
                
                # Record traversal step event if recorder available
                if self.event_recorder:
                    step_event = TraversalStepEvent(
                        traversal_id=self.traversal_id,
                        step_number=current_step_number,
                        execution_id=execution_id,
                        function_id="unknown",  # Will be enriched from graph
                        parent_execution_id=parent_execution_id,
                        duration_seconds=duration,
                        depth_level=current_depth,
                        sibling_count=sibling_count,
                        status="completed",
                        error_message=None,
                    )
                    step_event.information_content = step_event.compute_fisher_information()
                    
                    # Record event (async, but fire-and-forget)
                    try:
                        await self.event_recorder.record_traversal_step(step_event)
                    except Exception as e:
                        logger.error(f"Failed to record traversal step event: {e}")
                
                # Fetch children (still under semaphore)
                children = await self._get_children(execution_id)
                
                # Assign step numbers to children
                for child in children:
                    if child.step_number < 0:
                        child.step_number = self._step_number
                        self._step_number += 1
                
                progress.total_steps += len(children)
                
                # Update depth map for children
                if execution_id not in self.depth_map:
                    self.depth_map[execution_id] = current_depth + 1
                
                # Notify progress
                if config.progress_callback:
                    await config.progress_callback(progress)
            
            except Exception as e:
                progress.failed_steps += 1
                logger.error(f"Traversal error: {e}")
                children = []
        
        # Process children AFTER releasing semaphore to avoid deadlock
        if children:
            try:
                tasks = [
                    self._traverse_recursive(
                        child.execution_id,
                        config,
                        progress,
                        semaphore,
                        depth=depth + 1,
                        parent_execution_id=execution_id,
                        step_number=child.step_number,
                    )
                    for child in children
                ]
                await asyncio.gather(*tasks, return_exceptions=True)
            except Exception as e:
                progress.failed_steps += 1
                logger.error(f"Failed to traverse children of {execution_id}: {e}")
    
    async def get_children(self, execution_id: str) -> List[ExecutionStep]:
        """Fetch child executions."""
        return await self._get_children(execution_id)


class TraversalEngine:
    """Main execution traversal orchestrator.
    
    Coordinates graph traversal with policy enforcement, timing simulation,
    and progress streaming.
    """
    
    def __init__(
        self,
        execution_graph: Any,  # ExecutionLinkGraph instance
        timing_model: Optional[TimingModel] = None,
        event_recorder: Optional[Any] = None,  # InvocationEventRecorder instance
        clock: Optional[Any] = None,  # StepClock or WallClock for testing
    ):
        """Initialize traversal engine.
        
        Args:
            execution_graph: ExecutionLinkGraph instance
            timing_model: Optional custom timing model
            event_recorder: Optional InvocationEventRecorder for provenance tracking
            clock: Optional pluggable clock (StepClock for testing, WallClock for production)
        """
        from modelado.core.config import ClockFactory
        
        self.graph = execution_graph
        self.timing_model = timing_model or TimingModel()
        self.event_recorder = event_recorder
        self.clock = clock or ClockFactory.create()
        self._traversals: Dict[str, TraversalProgress] = {}
    
    async def traverse_breadth_first(
        self,
        root_execution_id: str,
        max_workers: int = 4,
        timeout_seconds: Optional[float] = None,
        progress_callback: Optional[Callable[[TraversalProgress], Coroutine]] = None,
    ) -> TraversalProgress:
        """Traverse graph breadth-first with bounded concurrency.
        
        Args:
            root_execution_id: Starting execution
            max_workers: Max concurrent workers
            timeout_seconds: Overall timeout
            progress_callback: Async callback for progress updates
            
        Returns:
            Final progress metrics
        """
        traverser = BreadthFirstTraverser(
            self._get_callee_steps,
            event_recorder=self.event_recorder,
        )
        
        config = TraversalConfig(
            order=TraversalOrder.BREADTH_FIRST,
            max_workers=max_workers,
            timing_model=self.timing_model,
            timeout_seconds=timeout_seconds,
            progress_callback=progress_callback,
        )
        
        progress = await traverser.traverse(root_execution_id, config)
        self._traversals[f"{root_execution_id}-bfs"] = progress
        return progress
    
    async def traverse_depth_first(
        self,
        root_execution_id: str,
        max_workers: int = 4,
        max_depth: Optional[int] = None,
        timeout_seconds: Optional[float] = None,
        progress_callback: Optional[Callable[[TraversalProgress], Coroutine]] = None,
    ) -> TraversalProgress:
        """Traverse graph depth-first with bounded concurrency.
        
        Args:
            root_execution_id: Starting execution
            max_workers: Max concurrent workers
            max_depth: Max traversal depth
            timeout_seconds: Overall timeout
            progress_callback: Async callback for progress updates
            
        Returns:
            Final progress metrics
        """
        traverser = DepthFirstTraverser(
            self._get_callee_steps,
            event_recorder=self.event_recorder,
        )
        
        config = TraversalConfig(
            order=TraversalOrder.DEPTH_FIRST,
            max_workers=max_workers,
            timing_model=self.timing_model,
            max_depth=max_depth,
            timeout_seconds=timeout_seconds,
            progress_callback=progress_callback,
        )
        
        progress = await traverser.traverse(root_execution_id, config)
        self._traversals[f"{root_execution_id}-dfs"] = progress
        return progress
    
    async def _get_callee_steps(self, execution_id: str) -> List[ExecutionStep]:
        """Fetch callee execution steps for given execution.
        
        Args:
            execution_id: Parent execution ID
            
        Returns:
            List of ExecutionStep objects for callees
        """
        # This would call into ExecutionLinkGraph to get children
        # For now, return empty list (will be integrated in Task 7.9)
        callees = await self.graph.get_callee_executions(execution_id)
        
        steps = []
        for i, callee_link in enumerate(callees):
            step = ExecutionStep(
                step_number=-1,  # Will be assigned by traverser
                execution_id=callee_link.callee_execution_id,
                function_id=callee_link.callee_function_id,
                parent_execution_id=execution_id,
            )
            steps.append(step)
        
        return steps
    
    def get_traversal_progress(self, execution_id: str, order: str = "bfs") -> Optional[TraversalProgress]:
        """Get progress for previous traversal.
        
        Args:
            execution_id: Root execution ID
            order: "bfs" or "dfs"
            
        Returns:
            TraversalProgress if traversal exists, else None
        """
        key = f"{execution_id}-{order}"
        return self._traversals.get(key)
