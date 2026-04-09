"""Shared schemas for interactions, orchestration, and executor contracts.

This package exports two broad groups of contracts:

- existing interaction/thread/deliberation/client models used by interacciones APIs
- orchestration contracts for workflows, execution requests/results, executor
  declarations, approvals, and trace persistence policy

The intent is to keep this package dependency-light so both `modelado` and
runtime `interacciones` packages can share a single contract layer.
"""

from .schemas import (
    InteractionType,
    InteractionScopeType,
    InteractionIn,
    InteractionOut,
    ArtifactDescriptor,
    ChatMessage,
    InteractionResponse,
)
from .threads import (
    Thread,
    ThreadCreate,
    ThreadUpdate,
    ThreadList,
    ThreadArchive,
    ThreadBase,
)
from .deliberation import (
    DeliberationEnvelope,
    DeliberationEvidence,
    DeliberationEvidenceKind,
    DeliberationPhase,
    DeliberationStatus,
    build_deliberation_envelope,
    build_deliberation_system_event,
)
from .decision_trace import (
    DecisionTrace,
    DecisionTraceCandidate,
    DecisionTraceQuery,
    coerce_decision_trace,
    decision_trace_to_json,
)
from .execution import (
    ApprovalRequested,
    ApprovalResolved,
    ExecutionCompleted,
    ExecutionFailed,
    ExecutionProgress,
    ExecutionQueueRequest,
    ExecutionQueued,
    ExecutionRequest,
    ExecutionScope,
    ResolutionMode,
)
from .executors import ExecutorDeclaration
from .operator_boundaries import OperatorBoundaries, OperatorBoundarySpec
from .topics import OrchestrationTopicNames
from .traces import TracePersistenceMode, TracePersistencePolicy
from .petri import (
    RichPetriArc,
    RichPetriPlace,
    RichPetriTransition,
    RichPetriWorkflow,
    SourceWorkflowStorageMode,
    SourceWorkflowStoragePolicy,
)
from .workflows import WorkflowDefinition, WorkflowLink, WorkflowNode, WorkflowPublishTarget
from .client import InteractionsClient

__all__ = [
    "InteractionType",
    "InteractionScopeType",
    "InteractionIn",
    "InteractionOut",
    "ArtifactDescriptor",
    "ChatMessage",
    "InteractionResponse",
    "Thread",
    "ThreadBase",
    "ThreadCreate",
    "ThreadUpdate",
    "ThreadList",
    "ThreadArchive",
    "DeliberationEnvelope",
    "DeliberationEvidence",
    "DeliberationEvidenceKind",
    "DeliberationPhase",
    "DeliberationStatus",
    "build_deliberation_envelope",
    "build_deliberation_system_event",
    "InteractionsClient",
    "DecisionTrace",
    "DecisionTraceCandidate",
    "DecisionTraceQuery",
    "coerce_decision_trace",
    "decision_trace_to_json",
    "ApprovalRequested",
    "ApprovalResolved",
    "ExecutionCompleted",
    "ExecutionFailed",
    "ExecutionProgress",
    "ExecutionQueueRequest",
    "ExecutionQueued",
    "ExecutionRequest",
    "ExecutionScope",
    "ResolutionMode",
    "ExecutorDeclaration",
    "OperatorBoundaries",
    "OperatorBoundarySpec",
    "OrchestrationTopicNames",
    "TracePersistenceMode",
    "TracePersistencePolicy",
    "RichPetriArc",
    "RichPetriPlace",
    "RichPetriTransition",
    "RichPetriWorkflow",
    "SourceWorkflowStorageMode",
    "SourceWorkflowStoragePolicy",
    "WorkflowDefinition",
    "WorkflowLink",
    "WorkflowNode",
    "WorkflowPublishTarget",
]
