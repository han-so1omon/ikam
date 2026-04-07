"""
Test Sequencing Framework for Multi-Step Workflows

Provides a declarative DSL for defining, executing, and reporting on
multi-step test sequences with:
- State tracking across steps
- Dependency management
- Automatic rollback/cleanup
- Detailed execution reporting
- Pre-built templates for common workflows

Quick Start:
    from modelado.testing import TestSequence, SequenceContext
    
    seq = (TestSequence("my_workflow")
        .step("step1", my_func1)
        .step("step2", my_func2, depends_on="step1")
        .cleanup("cleanup", cleanup_func)
    )
    
    context = SequenceContext(name="my_workflow")
    report = seq.execute(context)
    print(report.report())
"""

from modelado.testing.sequencer import (
    SequenceContext,
    SequenceReport,
    StepResult,
    StepStatus,
    TestSequence,
)

from modelado.testing.templates import (
    ArtifactLifecycleTemplate,
    BatchExtractionTemplate,
    ConceptExtractionTemplate,
    DerivationChainTemplate,
    FragmentHierarchyTemplate,
    RoundTripTemplate,
    SequenceTemplate,
)

__all__ = [
    # Core classes
    "SequenceContext",
    "SequenceReport",
    "StepResult",
    "StepStatus",
    "TestSequence",
    # Templates
    "SequenceTemplate",
    "ArtifactLifecycleTemplate",
    "BatchExtractionTemplate",
    "ConceptExtractionTemplate",
    "DerivationChainTemplate",
    "FragmentHierarchyTemplate",
    "RoundTripTemplate",
]
