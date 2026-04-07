"""
Pre-built Sequence Templates for Common IKAM Workflows

Provides reusable sequence templates for common testing patterns:
- Artifact lifecycle (create → decompose → store → reconstruct)
- Batch extraction (extract → validate → store → report)
- Derivation chain (source → transform → render → export)
- Fragment hierarchy (create → link → query → cleanup)

Example:
    # Use artifact lifecycle template
    template = ArtifactLifecycleTemplate(
        create_func=my_create_artifact,
        decompose_func=my_decompose,
        store_func=my_store,
    )
    
    sequence = template.build()
    result = sequence.execute(context)
"""

from __future__ import annotations

from typing import Callable, Optional

from modelado.testing.sequencer import SequenceContext, TestSequence


class SequenceTemplate:
    """Base class for reusable sequence templates."""
    
    def __init__(self, name: str):
        """Initialize template.
        
        Args:
            name: Template name
        """
        self.name = name
    
    def build(self) -> TestSequence:
        """Build the sequence. Subclasses override to define steps."""
        raise NotImplementedError


class ArtifactLifecycleTemplate(SequenceTemplate):
    """Template for artifact create → decompose → store → reconstruct workflow."""
    
    def __init__(
        self,
        create_func: Callable,
        decompose_func: Callable,
        store_func: Callable,
        reconstruct_func: Callable,
        cleanup_func: Optional[Callable] = None,
    ):
        """Initialize artifact lifecycle template.
        
        Args:
            create_func: Function to create artifact. Signature: (context) -> artifact
            decompose_func: Function to decompose. Signature: (context) -> fragments
            store_func: Function to store fragments. Signature: (context) -> None
            reconstruct_func: Function to reconstruct. Signature: (context) -> reconstructed
            cleanup_func: Optional cleanup function. Signature: (context) -> None
        """
        super().__init__("artifact_lifecycle")
        self.create_func = create_func
        self.decompose_func = decompose_func
        self.store_func = store_func
        self.reconstruct_func = reconstruct_func
        self.cleanup_func = cleanup_func
    
    def build(self) -> TestSequence:
        """Build artifact lifecycle sequence."""
        seq = TestSequence(self.name)
        
        seq.step("create", self.create_func)
        seq.step("decompose", self.decompose_func, depends_on="create")
        seq.step("store", self.store_func, depends_on="decompose")
        seq.step("reconstruct", self.reconstruct_func, depends_on="store")
        
        if self.cleanup_func:
            seq.cleanup("cleanup", self.cleanup_func)
        
        return seq


class BatchExtractionTemplate(SequenceTemplate):
    """Template for batch extraction workflow."""
    
    def __init__(
        self,
        enqueue_func: Callable,
        process_func: Callable,
        validate_func: Callable,
        store_func: Callable,
        cleanup_func: Optional[Callable] = None,
    ):
        """Initialize batch extraction template.
        
        Args:
            enqueue_func: Enqueue batch for processing. Signature: (context) -> batch_id
            process_func: Process the batch. Signature: (context) -> results
            validate_func: Validate processing results. Signature: (context) -> validation_report
            store_func: Store results. Signature: (context) -> None
            cleanup_func: Optional cleanup. Signature: (context) -> None
        """
        super().__init__("batch_extraction")
        self.enqueue_func = enqueue_func
        self.process_func = process_func
        self.validate_func = validate_func
        self.store_func = store_func
        self.cleanup_func = cleanup_func
    
    def build(self) -> TestSequence:
        """Build batch extraction sequence."""
        seq = TestSequence(self.name)
        
        seq.step("enqueue", self.enqueue_func)
        seq.step("process", self.process_func, depends_on="enqueue")
        seq.step("validate", self.validate_func, depends_on="process")
        seq.step("store", self.store_func, depends_on="validate")
        
        if self.cleanup_func:
            seq.cleanup("cleanup", self.cleanup_func)
        
        return seq


class DerivationChainTemplate(SequenceTemplate):
    """Template for derivation chain workflow."""
    
    def __init__(
        self,
        create_source_func: Callable,
        create_derivation_func: Callable,
        execute_derivation_func: Callable,
        render_func: Callable,
        export_func: Callable,
        cleanup_func: Optional[Callable] = None,
    ):
        """Initialize derivation chain template.
        
        Args:
            create_source_func: Create source artifact. Signature: (context) -> artifact
            create_derivation_func: Create derivation. Signature: (context) -> derivation
            execute_derivation_func: Execute derivation. Signature: (context) -> result
            render_func: Render result. Signature: (context) -> rendered
            export_func: Export to target format. Signature: (context) -> export_file
            cleanup_func: Optional cleanup. Signature: (context) -> None
        """
        super().__init__("derivation_chain")
        self.create_source_func = create_source_func
        self.create_derivation_func = create_derivation_func
        self.execute_derivation_func = execute_derivation_func
        self.render_func = render_func
        self.export_func = export_func
        self.cleanup_func = cleanup_func
    
    def build(self) -> TestSequence:
        """Build derivation chain sequence."""
        seq = TestSequence(self.name)
        
        seq.step("create_source", self.create_source_func)
        seq.step("create_derivation", self.create_derivation_func, depends_on="create_source")
        seq.step("execute", self.execute_derivation_func, depends_on="create_derivation")
        seq.step("render", self.render_func, depends_on="execute")
        seq.step("export", self.export_func, depends_on="render")
        
        if self.cleanup_func:
            seq.cleanup("cleanup", self.cleanup_func)
        
        return seq


class FragmentHierarchyTemplate(SequenceTemplate):
    """Template for fragment hierarchy operations."""
    
    def __init__(
        self,
        create_artifact_func: Callable,
        create_l0_func: Callable,
        create_l1_func: Callable,
        create_l2_func: Callable,
        link_hierarchy_func: Callable,
        query_func: Callable,
        cleanup_func: Optional[Callable] = None,
    ):
        """Initialize fragment hierarchy template.
        
        Args:
            create_artifact_func: Create artifact. Signature: (context) -> artifact
            create_l0_func: Create L0 fragments. Signature: (context) -> fragments
            create_l1_func: Create L1 fragments. Signature: (context) -> fragments
            create_l2_func: Create L2 fragments. Signature: (context) -> fragments
            link_hierarchy_func: Link parent-child relationships. Signature: (context) -> None
            query_func: Query hierarchy. Signature: (context) -> results
            cleanup_func: Optional cleanup. Signature: (context) -> None
        """
        super().__init__("fragment_hierarchy")
        self.create_artifact_func = create_artifact_func
        self.create_l0_func = create_l0_func
        self.create_l1_func = create_l1_func
        self.create_l2_func = create_l2_func
        self.link_hierarchy_func = link_hierarchy_func
        self.query_func = query_func
        self.cleanup_func = cleanup_func
    
    def build(self) -> TestSequence:
        """Build fragment hierarchy sequence."""
        seq = TestSequence(self.name)
        
        seq.step("create_artifact", self.create_artifact_func)
        seq.step("create_l0", self.create_l0_func, depends_on="create_artifact")
        seq.step("create_l1", self.create_l1_func, depends_on="create_l0")
        seq.step("create_l2", self.create_l2_func, depends_on="create_l1")
        seq.step("link_hierarchy", self.link_hierarchy_func, depends_on="create_l2")
        seq.step("query", self.query_func, depends_on="link_hierarchy")
        
        if self.cleanup_func:
            seq.cleanup("cleanup", self.cleanup_func)
        
        return seq


class ConceptExtractionTemplate(SequenceTemplate):
    """Template for concept extraction workflow."""
    
    def __init__(
        self,
        prepare_text_func: Callable,
        extract_func: Callable,
        validate_concepts_func: Callable,
        store_func: Callable,
        link_artifacts_func: Callable,
        cleanup_func: Optional[Callable] = None,
    ):
        """Initialize concept extraction template.
        
        Args:
            prepare_text_func: Prepare input text. Signature: (context) -> prepared_text
            extract_func: Extract concepts. Signature: (context) -> concepts
            validate_concepts_func: Validate extracted concepts. Signature: (context) -> validation
            store_func: Store concepts. Signature: (context) -> None
            link_artifacts_func: Link concepts to artifacts. Signature: (context) -> None
            cleanup_func: Optional cleanup. Signature: (context) -> None
        """
        super().__init__("concept_extraction")
        self.prepare_text_func = prepare_text_func
        self.extract_func = extract_func
        self.validate_concepts_func = validate_concepts_func
        self.store_func = store_func
        self.link_artifacts_func = link_artifacts_func
        self.cleanup_func = cleanup_func
    
    def build(self) -> TestSequence:
        """Build concept extraction sequence."""
        seq = TestSequence(self.name)
        
        seq.step("prepare", self.prepare_text_func)
        seq.step("extract", self.extract_func, depends_on="prepare")
        seq.step("validate", self.validate_concepts_func, depends_on="extract")
        seq.step("store", self.store_func, depends_on="validate")
        seq.step("link", self.link_artifacts_func, depends_on="store")
        
        if self.cleanup_func:
            seq.cleanup("cleanup", self.cleanup_func)
        
        return seq


class RoundTripTemplate(SequenceTemplate):
    """Template for round-trip testing (decompose → reconstruct)."""
    
    def __init__(
        self,
        create_artifact_func: Callable,
        decompose_func: Callable,
        reconstruct_func: Callable,
        assert_equality_func: Callable,
        cleanup_func: Optional[Callable] = None,
    ):
        """Initialize round-trip template.
        
        Args:
            create_artifact_func: Create original artifact. Signature: (context) -> artifact
            decompose_func: Decompose artifact. Signature: (context) -> fragments
            reconstruct_func: Reconstruct from fragments. Signature: (context) -> reconstructed
            assert_equality_func: Assert original == reconstructed. Signature: (context) -> bool
            cleanup_func: Optional cleanup. Signature: (context) -> None
        """
        super().__init__("round_trip")
        self.create_artifact_func = create_artifact_func
        self.decompose_func = decompose_func
        self.reconstruct_func = reconstruct_func
        self.assert_equality_func = assert_equality_func
        self.cleanup_func = cleanup_func
    
    def build(self) -> TestSequence:
        """Build round-trip sequence."""
        seq = TestSequence(self.name)
        
        seq.step("create", self.create_artifact_func)
        seq.step("decompose", self.decompose_func, depends_on="create")
        seq.step("reconstruct", self.reconstruct_func, depends_on="decompose")
        seq.step("assert_equality", self.assert_equality_func, depends_on="reconstruct")
        
        if self.cleanup_func:
            seq.cleanup("cleanup", self.cleanup_func)
        
        return seq
