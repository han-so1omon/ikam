"""
Tests for the Test Sequencing Framework

Validates:
1. Synchronous step execution
2. Asynchronous step execution
3. Dependency tracking
4. State passing between steps
5. Error handling and rollback
6. Cleanup execution in reverse order
7. Detailed reporting
"""

import asyncio
import pytest

from modelado.testing.sequencer import (
    SequenceContext,
    StepStatus,
    TestSequence,
)
from modelado.testing.templates import (
    ArtifactLifecycleTemplate,
    RoundTripTemplate,
    BatchExtractionTemplate,
    DerivationChainTemplate,
)


# ============================================================================
# Test Fixtures
# ============================================================================

@pytest.fixture
def simple_context():
    """Create a simple test context."""
    return SequenceContext(name="test_sequence")


# ============================================================================
# Synchronous Execution Tests
# ============================================================================

class TestSyncExecution:
    """Test synchronous sequence execution."""
    
    def test_single_step_success(self, simple_context):
        """Single step should execute and succeed."""
        def step1(ctx):
            ctx.set("result", "completed")
            return "done"
        
        seq = TestSequence("single_step").step("step1", step1)
        report = seq.execute(simple_context)
        
        assert report.status == StepStatus.SUCCESS
        assert len(report.steps) == 1
        assert report.steps[0].status == StepStatus.SUCCESS
        assert simple_context.get("result") == "completed"
    
    def test_multiple_steps_success(self, simple_context):
        """Multiple steps should execute in order."""
        def step1(ctx):
            ctx.set("step1", True)
            return 10
        
        def step2(ctx):
            ctx.set("step2", True)
            return ctx.get_step_output("step1") + 5
        
        def step3(ctx):
            ctx.set("step3", True)
            return ctx.get_step_output("step2") * 2
        
        seq = (TestSequence("multi_step")
            .step("step1", step1)
            .step("step2", step2)
            .step("step3", step3)
        )
        report = seq.execute(simple_context)
        
        assert report.status == StepStatus.SUCCESS
        assert len(report.steps) == 3
        assert all(s.status == StepStatus.SUCCESS for s in report.steps)
        assert simple_context.get("step1") is True
        assert simple_context.get("step2") is True
        assert simple_context.get("step3") is True
    
    def test_step_failure_stops_sequence(self, simple_context):
        """Step failure should stop sequence execution."""
        def step1(ctx):
            return "ok"
        
        def step2(ctx):
            raise ValueError("step2 failed")
        
        def step3(ctx):
            # Should not execute
            return "should_not_reach"
        
        seq = (TestSequence("with_failure")
            .step("step1", step1)
            .step("step2", step2)
            .step("step3", step3)
        )
        report = seq.execute(simple_context)
        
        assert report.status == StepStatus.FAILED
        assert report.failed_step == "step2"
        assert len(report.steps) == 3  # Step3 recorded as skipped after failure
        assert report.steps[0].status == StepStatus.SUCCESS
        assert report.steps[1].status == StepStatus.FAILED
        assert report.steps[2].status == StepStatus.SKIPPED
    
    def test_dependency_tracking(self, simple_context):
        """Dependent steps should skip if dependency fails."""
        def step1(ctx):
            raise ValueError("step1 failed")
        
        def step2(ctx):
            return "should_not_reach"
        
        seq = (TestSequence("with_dependency")
            .step("step1", step1)
            .step("step2", step2, depends_on="step1")
        )
        report = seq.execute(simple_context)
        
        assert report.status == StepStatus.FAILED
        assert report.steps[0].status == StepStatus.FAILED
        assert report.steps[1].status == StepStatus.SKIPPED
        assert "Dependency 'step1' not completed" in report.steps[1].error_traceback
    
    def test_cleanup_on_success(self, simple_context):
        """Cleanup should execute on successful completion."""
        cleanup_called = []
        
        def step1(ctx):
            ctx.set("executed", True)
        
        def cleanup(ctx):
            cleanup_called.append("cleanup")
        
        seq = (TestSequence("with_cleanup")
            .step("step1", step1)
            .cleanup("cleanup", cleanup)
        )
        report = seq.execute(simple_context)
        
        assert report.status == StepStatus.SUCCESS
        assert cleanup_called == ["cleanup"]
    
    def test_cleanup_on_failure(self, simple_context):
        """Cleanup should execute even on failure."""
        cleanup_called = []
        
        def step1(ctx):
            raise ValueError("step1 failed")
        
        def cleanup(ctx):
            cleanup_called.append("cleanup")
        
        seq = (TestSequence("with_cleanup_on_failure")
            .step("step1", step1)
            .cleanup("cleanup", cleanup)
        )
        report = seq.execute(simple_context)
        
        assert report.status == StepStatus.FAILED
        assert cleanup_called == ["cleanup"]
    
    def test_cleanup_reverse_order(self, simple_context):
        """Cleanup steps should execute in reverse order."""
        cleanup_order = []
        
        def step1(ctx):
            pass
        
        def cleanup1(ctx):
            cleanup_order.append(1)
        
        def cleanup2(ctx):
            cleanup_order.append(2)
        
        def cleanup3(ctx):
            cleanup_order.append(3)
        
        seq = (TestSequence("cleanup_order")
            .step("step1", step1)
            .cleanup("cleanup1", cleanup1)
            .cleanup("cleanup2", cleanup2)
            .cleanup("cleanup3", cleanup3)
        )
        report = seq.execute(simple_context)
        
        assert cleanup_order == [3, 2, 1]
    
    def test_state_passing(self, simple_context):
        """State should be accessible across steps."""
        def step1(ctx):
            ctx.set("value", 42)
            ctx.set("name", "test")
            return {"step1": True}
        
        def step2(ctx):
            assert ctx.get("value") == 42
            assert ctx.get("name") == "test"
            return {"step2": True}
        
        seq = (TestSequence("state_passing")
            .step("step1", step1)
            .step("step2", step2, depends_on="step1")
        )
        report = seq.execute(simple_context)
        
        assert report.status == StepStatus.SUCCESS


# ============================================================================
# Asynchronous Execution Tests
# ============================================================================

class TestAsyncExecution:
    """Test asynchronous sequence execution."""
    
    @pytest.mark.asyncio
    async def test_async_single_step(self, simple_context):
        """Single async step should execute."""
        async def async_step1(ctx):
            await asyncio.sleep(0.01)
            ctx.set("async", True)
            return "async_done"
        
        seq = TestSequence("async_single").step("async_step1", async_step1)
        report = await seq.execute_async(simple_context)
        
        assert report.status == StepStatus.SUCCESS
        assert simple_context.get("async") is True
    
    @pytest.mark.asyncio
    async def test_async_multiple_steps(self, simple_context):
        """Multiple async steps should execute in order."""
        async def async_step1(ctx):
            await asyncio.sleep(0.01)
            return 10
        
        async def async_step2(ctx):
            await asyncio.sleep(0.01)
            return ctx.get_step_output("async_step1") + 5
        
        seq = (TestSequence("async_multi")
            .step("async_step1", async_step1)
            .step("async_step2", async_step2, depends_on="async_step1")
        )
        report = await seq.execute_async(simple_context)
        
        assert report.status == StepStatus.SUCCESS
        assert len(report.steps) == 2
    
    @pytest.mark.asyncio
    async def test_async_cleanup(self, simple_context):
        """Async cleanup should execute."""
        cleanup_called = []
        
        async def async_step(ctx):
            await asyncio.sleep(0.01)
        
        async def async_cleanup(ctx):
            await asyncio.sleep(0.01)
            cleanup_called.append("cleanup")
        
        seq = (TestSequence("async_cleanup")
            .step("async_step", async_step)
            .cleanup("async_cleanup", async_cleanup)
        )
        report = await seq.execute_async(simple_context)
        
        assert cleanup_called == ["cleanup"]


# ============================================================================
# Reporting Tests
# ============================================================================

class TestReporting:
    """Test sequence reporting."""
    
    def test_report_success(self, simple_context):
        """Success report should show all steps."""
        def step1(ctx):
            return "result1"
        
        def step2(ctx):
            return "result2"
        
        seq = (TestSequence("reporting_test")
            .step("step1", step1)
            .step("step2", step2, depends_on="step1")
        )
        report = seq.execute(simple_context)
        
        assert report.status == StepStatus.SUCCESS
        assert report.success_rate() == 100.0
        report_text = report.report()
        assert "Reporting test" in report_text
        assert "✅ step1" in report_text
        assert "✅ step2" in report_text
    
    def test_report_failure(self, simple_context):
        """Failure report should show failed step."""
        def step1(ctx):
            return "ok"
        
        def step2(ctx):
            raise ValueError("step2 error")
        
        seq = (TestSequence("failure_report")
            .step("step1", step1)
            .step("step2", step2, depends_on="step1")
        )
        report = seq.execute(simple_context)
        
        assert report.status == StepStatus.FAILED
        assert report.success_rate() < 100.0
        report_text = report.report()
        assert "❌ step2" in report_text
        assert "step2 error" in report_text
    
    def test_report_dict_serialization(self, simple_context):
        """Report should serialize to dict."""
        def step1(ctx):
            return "result"
        
        seq = TestSequence("serialization").step("step1", step1)
        report = seq.execute(simple_context)
        
        report_dict = report.to_dict()
        assert report_dict["name"] == "serialization"
        assert report_dict["status"] == "success"
        assert "success_rate" in report_dict
        assert len(report_dict["steps"]) == 1


# ============================================================================
# Template Tests
# ============================================================================

class TestTemplates:
    """Test pre-built sequence templates."""
    
    def test_artifact_lifecycle_template(self, simple_context):
        """Artifact lifecycle template should work."""
        def create(ctx):
            ctx.set("artifact", {"id": "art1", "content": "test"})
            return ctx.get("artifact")
        
        def decompose(ctx):
            artifact = ctx.get_step_output("create")
            ctx.set("fragments", ["frag1", "frag2"])
            return ctx.get("fragments")
        
        def store(ctx):
            fragments = ctx.get_step_output("decompose")
            ctx.set("stored", True)
        
        def reconstruct(ctx):
            return ctx.get_step_output("create")
        
        template = ArtifactLifecycleTemplate(
            create_func=create,
            decompose_func=decompose,
            store_func=store,
            reconstruct_func=reconstruct,
        )
        
        seq = template.build()
        report = seq.execute(simple_context)
        
        assert report.status == StepStatus.SUCCESS
        assert len(report.steps) == 4
    
    def test_round_trip_template(self, simple_context):
        """Round-trip template should validate reconstruction."""
        original_content = b"test content"
        
        def create(ctx):
            ctx.set("original", original_content)
            return original_content
        
        def decompose(ctx):
            original = ctx.get_step_output("create")
            ctx.set("fragments", [original[:5], original[5:]])
            return ctx.get("fragments")
        
        def reconstruct(ctx):
            fragments = ctx.get_step_output("decompose")
            reconstructed = b"".join(fragments)
            ctx.set("reconstructed", reconstructed)
            return reconstructed
        
        def assert_equality(ctx):
            original = ctx.get_step_output("create")
            reconstructed = ctx.get_step_output("reconstruct")
            assert original == reconstructed
        
        template = RoundTripTemplate(
            create_artifact_func=create,
            decompose_func=decompose,
            reconstruct_func=reconstruct,
            assert_equality_func=assert_equality,
        )
        
        seq = template.build()
        report = seq.execute(simple_context)
        
        assert report.status == StepStatus.SUCCESS
    
    def test_batch_extraction_template(self, simple_context):
        """Batch extraction template should work."""
        def enqueue(ctx):
            ctx.set("batch_id", "batch123")
            return "batch123"
        
        def process(ctx):
            batch_id = ctx.get_step_output("enqueue")
            ctx.set("results", ["result1", "result2"])
            return ctx.get("results")
        
        def validate(ctx):
            results = ctx.get_step_output("process")
            assert len(results) > 0
            return True
        
        def store(ctx):
            pass
        
        template = BatchExtractionTemplate(
            enqueue_func=enqueue,
            process_func=process,
            validate_func=validate,
            store_func=store,
        )
        
        seq = template.build()
        report = seq.execute(simple_context)
        
        assert report.status == StepStatus.SUCCESS


# ============================================================================
# Edge Cases and Error Handling
# ============================================================================

class TestEdgeCases:
    """Test edge cases and error scenarios."""
    
    def test_empty_sequence(self, simple_context):
        """Empty sequence should complete immediately."""
        seq = TestSequence("empty")
        report = seq.execute(simple_context)
        
        assert report.status == StepStatus.SUCCESS
        assert len(report.steps) == 0
    
    def test_cleanup_error_doesnt_stop_execution(self, simple_context):
        """Cleanup error should not prevent other cleanups."""
        cleanup_order = []
        
        def step1(ctx):
            pass
        
        def cleanup1(ctx):
            raise ValueError("cleanup1 failed")
        
        def cleanup2(ctx):
            cleanup_order.append(2)
        
        seq = (TestSequence("cleanup_error")
            .step("step1", step1)
            .cleanup("cleanup1", cleanup1)
            .cleanup("cleanup2", cleanup2)
        )
        report = seq.execute(simple_context)
        
        # Cleanup error should be logged but sequence should complete
        assert cleanup_order == [2]
    
    def test_missing_dependency(self, simple_context):
        """Step depending on non-existent step should skip."""
        def step1(ctx):
            return "ok"
        
        def step2(ctx):
            return "should_skip"
        
        seq = (TestSequence("missing_dep")
            .step("step1", step1)
            .step("step2", step2, depends_on="nonexistent")
        )
        report = seq.execute(simple_context)
        
        assert report.steps[1].status == StepStatus.SKIPPED
    
    def test_timing_measurement(self, simple_context):
        """Step timing should be measured."""
        import time
        
        def slow_step(ctx):
            time.sleep(0.05)
        
        seq = TestSequence("timing").step("slow", slow_step)
        report = seq.execute(simple_context)
        
        assert report.steps[0].duration_ms >= 50
        assert report.total_duration_ms >= 50
