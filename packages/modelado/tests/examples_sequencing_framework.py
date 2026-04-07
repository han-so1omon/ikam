"""
Example Usage of the Test Sequencing Framework

This module demonstrates practical examples of using the test sequencing
framework for IKAM workflows.
"""

from modelado.testing.sequencer import SequenceContext, TestSequence
from modelado.testing.templates import (
    ArtifactLifecycleTemplate,
    RoundTripTemplate,
    BatchExtractionTemplate,
    ConceptExtractionTemplate,
)


# ============================================================================
# Example 1: Simple Multi-Step Sequence
# ============================================================================

def example_simple_sequence():
    """Example: Simple artifact workflow."""
    
    def step_create_artifact(ctx: SequenceContext):
        """Create test artifact."""
        artifact = {
            "id": "artifact-1",
            "title": "Test Document",
            "content": "This is test content for IKAM processing.",
        }
        ctx.set("artifact", artifact)
        return artifact
    
    def step_validate(ctx: SequenceContext):
        """Validate artifact structure."""
        artifact = ctx.get_step_output("create_artifact")
        assert "id" in artifact
        assert "title" in artifact
        ctx.set("validation_passed", True)
        return True
    
    def step_prepare(ctx: SequenceContext):
        """Prepare artifact for processing."""
        artifact = ctx.get("artifact")
        prepared = {**artifact, "processed": True}
        ctx.set("prepared_artifact", prepared)
        return prepared
    
    # Build sequence
    seq = (TestSequence("artifact_workflow")
        .step("create_artifact", step_create_artifact)
        .step("validate", step_validate, depends_on="create_artifact")
        .step("prepare", step_prepare, depends_on="validate")
    )
    
    # Execute
    context = SequenceContext(name="artifact_workflow")
    report = seq.execute(context)
    
    # Print report
    print(report.report())
    print(f"\nFinal state: {context.state}")


# ============================================================================
# Example 2: Using Artifact Lifecycle Template
# ============================================================================

def example_artifact_lifecycle():
    """Example: Using artifact lifecycle template."""
    
    class MockArtifactService:
        """Mock service for artifact operations."""
        
        @staticmethod
        def create_artifact(ctx: SequenceContext):
            """Create artifact."""
            artifact = {
                "id": "art-123",
                "blocks": [
                    {"type": "heading", "text": "Title"},
                    {"type": "paragraph", "text": "Content"},
                ]
            }
            ctx.set("artifact", artifact)
            return artifact
        
        @staticmethod
        def decompose_artifact(ctx: SequenceContext):
            """Decompose into fragments."""
            artifact = ctx.get_step_output("create")
            fragments = [
                {"level": 0, "content": "Title"},
                {"level": 1, "content": "Content"},
            ]
            ctx.set("fragments", fragments)
            return fragments
        
        @staticmethod
        def store_fragments(ctx: SequenceContext):
            """Store fragments."""
            fragments = ctx.get_step_output("decompose")
            ctx.set("stored_fragment_count", len(fragments))
        
        @staticmethod
        def reconstruct_artifact(ctx: SequenceContext):
            """Reconstruct from fragments."""
            fragments = ctx.get_step_output("store")
            # In real scenario, would reconstruct from storage
            reconstructed = ctx.get("artifact")
            ctx.set("reconstructed", reconstructed)
            return reconstructed
        
        @staticmethod
        def cleanup(ctx: SequenceContext):
            """Cleanup resources."""
            ctx.set("cleaned_up", True)
    
    # Use template
    template = ArtifactLifecycleTemplate(
        create_func=MockArtifactService.create_artifact,
        decompose_func=MockArtifactService.decompose_artifact,
        store_func=MockArtifactService.store_fragments,
        reconstruct_func=MockArtifactService.reconstruct_artifact,
        cleanup_func=MockArtifactService.cleanup,
    )
    
    # Build and execute
    seq = template.build()
    context = SequenceContext(name="artifact_lifecycle")
    report = seq.execute(context)
    
    print("\n" + "="*70)
    print(report.report())
    print(f"\nContext state after execution:")
    for key, value in context.state.items():
        print(f"  {key}: {value}")


# ============================================================================
# Example 3: Round-Trip Testing
# ============================================================================

def example_round_trip():
    """Example: Round-trip decompose/reconstruct testing."""
    
    original_bytes = b"The quick brown fox jumps over the lazy dog."
    
    def create_document(ctx: SequenceContext):
        """Create document."""
        ctx.set("original_bytes", original_bytes)
        return original_bytes
    
    def decompose(ctx: SequenceContext):
        """Decompose document."""
        original = ctx.get_step_output("create")
        # Split into chunks
        chunk_size = 15
        fragments = [
            original[i:i+chunk_size] 
            for i in range(0, len(original), chunk_size)
        ]
        ctx.set("fragments", fragments)
        return fragments
    
    def reconstruct(ctx: SequenceContext):
        """Reconstruct from fragments."""
        fragments = ctx.get_step_output("decompose")
        reconstructed = b"".join(fragments)
        ctx.set("reconstructed_bytes", reconstructed)
        return reconstructed
    
    def verify_equality(ctx: SequenceContext):
        """Verify reconstruction matches original."""
        original = ctx.get_step_output("create")
        reconstructed = ctx.get_step_output("reconstruct")
        
        if original != reconstructed:
            raise AssertionError(
                f"Round-trip failed!\n"
                f"Original ({len(original)} bytes): {original}\n"
                f"Reconstructed ({len(reconstructed)} bytes): {reconstructed}"
            )
        
        return True
    
    # Use template
    template = RoundTripTemplate(
        create_artifact_func=create_document,
        decompose_func=decompose,
        reconstruct_func=reconstruct,
        assert_equality_func=verify_equality,
    )
    
    # Execute
    seq = template.build()
    context = SequenceContext(name="round_trip")
    report = seq.execute(context)
    
    print("\n" + "="*70)
    print(report.report())
    
    if report.status.value == "success":
        print("\n✅ Round-trip test passed!")
        print(f"Original:      {context.get('original_bytes')}")
        print(f"Reconstructed: {context.get('reconstructed_bytes')}")


# ============================================================================
# Example 4: Error Handling and Rollback
# ============================================================================

def example_error_handling():
    """Example: Error handling with cleanup."""
    
    resource_list = []
    
    def allocate_resource(ctx: SequenceContext):
        """Allocate resource."""
        resource_list.append("resource_1")
        ctx.set("allocated_resources", resource_list.copy())
        return resource_list[-1]
    
    def process_resource(ctx: SequenceContext):
        """Process resource - will fail."""
        resource = ctx.get_step_output("allocate")
        # Simulate failure
        raise RuntimeError(f"Failed to process {resource}")
    
    def cleanup_resources(ctx: SequenceContext):
        """Cleanup allocated resources."""
        resource_list.clear()
        ctx.set("cleanup_completed", True)
        print("  → Cleaning up resources...")
    
    # Build sequence with cleanup
    seq = (TestSequence("error_handling")
        .step("allocate", allocate_resource)
        .step("process", process_resource, depends_on="allocate")
        .cleanup("cleanup", cleanup_resources)
    )
    
    # Execute
    context = SequenceContext(name="error_handling")
    report = seq.execute(context)
    
    print("\n" + "="*70)
    print(report.report())
    print(f"\nResources after execution: {resource_list}")
    print(f"Cleanup was executed: {context.get('cleanup_completed')}")


# ============================================================================
# Example 5: Complex Dependency Chain
# ============================================================================

def example_complex_dependencies():
    """Example: Complex step dependencies."""
    
    def step_1(ctx: SequenceContext):
        """First step."""
        ctx.set("step_1_data", "value_1")
        print("  → Step 1 completed")
        return "output_1"
    
    def step_2(ctx: SequenceContext):
        """Second step - parallel with step 3."""
        output_1 = ctx.get_step_output("step_1")
        ctx.set("step_2_data", f"processed_{output_1}")
        print("  → Step 2 completed")
        return "output_2"
    
    def step_3(ctx: SequenceContext):
        """Third step - parallel with step 2."""
        output_1 = ctx.get_step_output("step_1")
        ctx.set("step_3_data", f"analyzed_{output_1}")
        print("  → Step 3 completed")
        return "output_3"
    
    def step_4(ctx: SequenceContext):
        """Fourth step - depends on both 2 and 3."""
        output_2 = ctx.get_step_output("step_2")
        output_3 = ctx.get_step_output("step_3")
        ctx.set("step_4_data", f"combined_{output_2}_{output_3}")
        print("  → Step 4 completed")
        return "output_4"
    
    # Build sequence with dependencies
    seq = (TestSequence("complex_dependencies")
        .step("step_1", step_1)
        .step("step_2", step_2, depends_on="step_1")
        .step("step_3", step_3, depends_on="step_1")
        .step("step_4", step_4, depends_on="step_2")  # Only depends on step_2
    )
    
    # Execute
    context = SequenceContext(name="complex_dependencies")
    report = seq.execute(context)
    
    print("\n" + "="*70)
    print(report.report())
    print(f"\nExecution timeline:")
    for i, step in enumerate(report.steps, 1):
        print(f"  {i}. {step.name}: {step.duration_ms:.2f}ms")


# ============================================================================
# Main
# ============================================================================

if __name__ == "__main__":
    print("\n" + "="*70)
    print("Test Sequencing Framework Examples")
    print("="*70)
    
    print("\n--- Example 1: Simple Sequence ---")
    example_simple_sequence()
    
    print("\n\n--- Example 2: Artifact Lifecycle Template ---")
    example_artifact_lifecycle()
    
    print("\n\n--- Example 3: Round-Trip Testing ---")
    example_round_trip()
    
    print("\n\n--- Example 4: Error Handling and Rollback ---")
    example_error_handling()
    
    print("\n\n--- Example 5: Complex Dependencies ---")
    example_complex_dependencies()
