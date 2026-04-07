"""IKAM Project core models

Comprehensive Pydantic models for IKAM Project orchestration, including:
- Full domain models: Project, Artifacts (Document, SlideDeck, Sheet, EconomicModel, StoryModel)
- Lightweight references: ArtifactRef, ModelRef, MediaRef
- Derivation graph: Derivation, DerivationGraph, impact analysis
- API contracts: DerivationCreate, DerivationOut, ImpactAnalysisOut
- Project metadata: ProjectMeta, TeamMember, ProjectSettings
- IKAM v3 Fragment Algebra: Fragment, Relation, codec, decomposition/reconstruction
"""

from .models import (
    # Project core models
    TeamMember,
    ProjectSettings,
    ProjectMeta,
    
    # Artifact models - Full domain models
    ArtifactStatus,
    ArtifactType,
    ContentRef,
    VersionRef,
    BaseArtifact,
    DocumentArtifact,
    SlideDeckArtifact,
    SheetArtifact,
    EconomicModelArtifact,
    StoryModelArtifact,
    ExternalDocumentArtifact,
    Artifact,
    ArtifactRegistry,
    
    # Lightweight references
    ArtifactRef,
    DocumentRef,
    SlideDeckRef,
    SheetRef,
    
    # Model & media registries
    ModelKind,
    ModelRef,
    MediaKind,
    MediaRef,
    
    # Derivation models
    DerivationType,
    Derivation,
    DerivationGraph,
    
    # Snapshots & instructions
    Snapshot,
    Instruction,
    
    # Project models
    ProjectLight,
    Project,
    
    # Service-layer API models
    DerivationCreate,
    DerivationOut,
    ImpactNode,
    ImpactAnalysisOut,
    RegenerationRequest,
    
    # Helper functions
    artifact_from_dict,
)

# IKAM Fragment Algebra V3 (canonical)
from .fragments import (
    Fragment,
    SlotBinding,
    BindingGroup,
    Relation,
    RELATION_MIME,
    is_relation_fragment,
)

# Configuration types (migrated from legacy_fragments)
from .config import (
    DecompositionStrategy,
    DecompositionConfig,
    ReconstructionConfig,
)

from .codec import (
    FragmentCodec,
    FragmentListCodec,
)

from .forja import (
    reconstruct_document,
    reconstruct_binary,
    DecompositionError,
    ReconstructionError,
)

# IKAM Sheet Models
from .sheet_models import (
    Cell,
    CellFormat,
    CellRange,
    CellStyle,
    CellType,
    CellValue,
    Chart,
    ChartSeries,
    ConditionalFormat,
    DataValidation,
    ErrorType,
    NamedRange,
    Sheet,
    SheetDimensions,
    SheetFragmentContent,
    Workbook,
    WorkbookMeta,
)

from .sheet_decomposition import (
    decompose_workbook,
    reconstruct_workbook,
    SheetDecompositionError,
    SheetReconstructionError,
)

from .inspection import (
    InspectionRef,
    InspectionNode,
    InspectionEdge,
    ResolveInspectionRequest,
    InspectionResolver,
    InspectionSubgraph,
)

__all__ = [
    # Project core
    "TeamMember",
    "ProjectSettings",
    "ProjectMeta",
    
    # Artifacts - full models
    "ArtifactStatus",
    "ArtifactType",
    "ContentRef",
    "VersionRef",
    "BaseArtifact",
    "DocumentArtifact",
    "SlideDeckArtifact",
    "SheetArtifact",
    "EconomicModelArtifact",
    "StoryModelArtifact",
    "ExternalDocumentArtifact",
    "Artifact",
    "ArtifactRegistry",
    
    # Lightweight references
    "ArtifactRef",
    "DocumentRef",
    "SlideDeckRef",
    "SheetRef",
    
    # Registries
    "ModelKind",
    "ModelRef",
    "MediaKind",
    "MediaRef",
    
    # Derivations
    "DerivationType",
    "Derivation",
    "DerivationGraph",
    
    # Snapshots & instructions
    "Snapshot",
    "Instruction",
    
    # Projects
    "ProjectLight",
    "Project",
    
    # API models
    "DerivationCreate",
    "DerivationOut",
    "ImpactNode",
    "ImpactAnalysisOut",
    "RegenerationRequest",
    
    # Helpers
    "artifact_from_dict",
    
    # IKAM Fragment Algebra V3
    "Fragment",
    "SlotBinding",
    "BindingGroup",
    "Relation",
    "RELATION_MIME",
    "is_relation_fragment",
    
    # Configuration
    "DecompositionStrategy",
    "DecompositionConfig",
    "ReconstructionConfig",
    
    # Codec
    "FragmentCodec",
    "FragmentListCodec",
    
    # Decomposition/Reconstruction
    "reconstruct_document",
    "reconstruct_binary",
    "DecompositionError",
    "ReconstructionError",
    
    # IKAM Sheet Models
    "Cell",
    "CellFormat",
    "CellRange",
    "CellStyle",
    "CellType",
    "CellValue",
    "Chart",
    "ChartSeries",
    "ConditionalFormat",
    "DataValidation",
    "ErrorType",
    "NamedRange",
    "Sheet",
    "SheetDimensions",
    "SheetFragmentContent",
    "Workbook",
    "WorkbookMeta",
    "decompose_workbook",
    "reconstruct_workbook",
    "SheetDecompositionError",
    "SheetReconstructionError",

    # Inspection schema
    "InspectionRef",
    "InspectionNode",
    "InspectionEdge",
    "ResolveInspectionRequest",
    "InspectionResolver",
    "InspectionSubgraph",
]
