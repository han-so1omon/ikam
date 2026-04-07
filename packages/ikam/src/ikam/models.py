"""
IKAM Project Domain Models

Comprehensive Pydantic models for IKAM Project specification (v1.0.0).
These models represent the canonical structure for projects, artifacts,
and the project graph in the Narraciones system.

References:
- docs/ikam/ikam-project-specification.md
- docs/ikam/ikam-specification.md (documents/slides)
- docs/ikam/ikam-sheet-specification.md (sheets)

This package consolidates all IKAM domain models, including:
- Project core models (TeamMember, ProjectSettings, ProjectMeta)
- Full artifact models (Document, SlideDeck, Sheet, EconomicModel, StoryModel)
- Lightweight artifact/model/media references
- Derivation graph and impact analysis
- API contract models for services
"""

from __future__ import annotations

from datetime import datetime
from enum import Enum
from typing import Any, Dict, List, Literal, Optional, Union

from pydantic import BaseModel, Field, ConfigDict, field_validator


# =============================================================================
# Project Core Models
# =============================================================================

class TeamMember(BaseModel):
    """Member of a project team with specific role."""
    user_id: str = Field(..., alias="userId")
    role: Literal["owner", "editor", "viewer"]
    joined_at: datetime = Field(default_factory=datetime.now, alias="joinedAt")

    model_config = ConfigDict(populate_by_name=True)


class ProjectSettings(BaseModel):
    """Project-level settings and preferences."""
    default_currency: Optional[str] = Field(None, alias="defaultCurrency")  # ISO 4217 (e.g., "USD")
    default_locale: Optional[str] = Field(None, alias="defaultLocale")  # e.g., "en-US"
    fiscal_year_start: Optional[str] = Field(None, alias="fiscalYearStart")  # "MM-DD" (e.g., "01-01")
    theme: Optional[str] = None  # Reference to theme

    model_config = ConfigDict(populate_by_name=True)


class ProjectMeta(BaseModel):
    """Project metadata following IKAM Project specification."""
    name: str
    description: Optional[str] = None
    industry: Optional[str] = None  # e.g., "SaaS", "Marketplace", "Hardware"
    stage: Optional[str] = None  # e.g., "Idea", "Seed", "Series A"
    team: List[TeamMember] = Field(default_factory=list)
    tags: List[str] = Field(default_factory=list)
    created_at: datetime = Field(default_factory=datetime.now, alias="createdAt")
    updated_at: datetime = Field(default_factory=datetime.now, alias="updatedAt")
    settings: Optional[ProjectSettings] = None

    model_config = ConfigDict(populate_by_name=True)


# =============================================================================
# Artifact Models - Full Domain Models
# =============================================================================

class ArtifactStatus(str, Enum):
    """Lifecycle status for artifacts."""
    DRAFT = "draft"
    REVIEW = "review"
    FINAL = "final"
    ARCHIVED = "archived"


class ArtifactType(str, Enum):
    """Discriminator for artifact types."""
    # IKAM native types
    IKAM_DOCUMENT = "ikam-document"
    IKAM_SLIDE_DECK = "ikam-slideDeck"
    IKAM_SHEET = "ikam-sheet"
    
    # Model types
    ECONOMIC_MODEL = "economic-model"
    STORY_MODEL = "story-model"
    
    # External/uploaded content
    EXTERNAL_DOCUMENT = "external-document"
    
    # Legacy/backward compat types (deprecated: prefer IKAM canonical types)
    SHEET = "sheet"
    STORY_SLIDES = "ikam-slideDeck"  # Alias for backward compatibility
    DOCUMENT = "document"
    UNIVER_SHEET = "univer-sheet"
    UNIVER_SLIDES = "univer-slides"
    
    # Lightweight reference types (for API contracts)
    document = "document"
    slide_deck = "slide-deck"
    sheet = "sheet"


class ContentRef(BaseModel):
    """Reference to artifact content storage location."""
    type: str  # Storage type: "database", "minio", "s3", "url"
    path: Optional[str] = None  # Storage path or key
    data: Optional[Dict[str, Any]] = None  # Inline content for database-backed artifacts
    meta: Optional[Dict[str, Any]] = None  # Storage-specific metadata

    model_config = ConfigDict(extra="allow")  # Allow type-specific fields


class VersionRef(BaseModel):
    """Reference to an artifact version."""
    version_id: str = Field(..., alias="versionId")
    version_number: int = Field(..., alias="versionNumber")
    created_at: datetime = Field(..., alias="createdAt")
    created_by: str = Field(..., alias="createdBy")
    label: Optional[str] = None
    
    model_config = ConfigDict(populate_by_name=True)


class BaseArtifact(BaseModel):
    """Base artifact model following IKAM Project specification."""
    id: str
    project_id: str = Field(..., alias="projectId")
    type: ArtifactType
    name: str
    description: Optional[str] = None
    status: ArtifactStatus = ArtifactStatus.DRAFT
    content_ref: ContentRef = Field(..., alias="contentRef")
    meta: Optional[Dict[str, Any]] = None
    created_at: datetime = Field(default_factory=datetime.now, alias="createdAt")
    updated_at: datetime = Field(default_factory=datetime.now, alias="updatedAt")
    created_by: str = Field(..., alias="createdBy")
    updated_by: Optional[str] = Field(None, alias="updatedBy")
    tags: List[str] = Field(default_factory=list)
    version_history: List[VersionRef] = Field(default_factory=list, alias="versionHistory")

    model_config = ConfigDict(populate_by_name=True)


class DocumentArtifact(BaseArtifact):
    """IKAM Document artifact."""
    type: Literal[ArtifactType.IKAM_DOCUMENT] = ArtifactType.IKAM_DOCUMENT
    format: Literal["ikam"] = "ikam"


class SlideDeckArtifact(BaseArtifact):
    """IKAM Slide Deck artifact."""
    type: Literal[ArtifactType.IKAM_SLIDE_DECK] = ArtifactType.IKAM_SLIDE_DECK
    format: Literal["ikam"] = "ikam"
    theme: Optional[str] = None  # ThemeReference


class SheetArtifact(BaseArtifact):
    """IKAM Sheet (workbook) artifact."""
    type: Literal[ArtifactType.IKAM_SHEET] = ArtifactType.IKAM_SHEET
    format: Literal["ikam"] = "ikam"
    sheets: List[str] = Field(default_factory=list)  # Sheet names within workbook


class EconomicModelArtifact(BaseArtifact):
    """Economic model artifact (financial projections, unit economics)."""
    type: Literal[ArtifactType.ECONOMIC_MODEL] = ArtifactType.ECONOMIC_MODEL
    inputs: Dict[str, Any] = Field(default_factory=dict)  # EconomicInputSchema
    outputs: Dict[str, Any] = Field(default_factory=dict)  # EconomicOutputSchema


class StoryModelArtifact(BaseArtifact):
    """Story model artifact (narrative structure, slide plans)."""
    type: Literal[ArtifactType.STORY_MODEL] = ArtifactType.STORY_MODEL
    slides: List[Dict[str, Any]] = Field(default_factory=list)  # SlideSchema[]


class ExternalDocumentArtifact(BaseArtifact):
    """External/uploaded document artifact (PDFs, images, etc.)."""
    type: Literal[ArtifactType.EXTERNAL_DOCUMENT] = ArtifactType.EXTERNAL_DOCUMENT
    mime_type: str = Field(..., alias="mimeType")
    file_size: int = Field(..., alias="fileSize")  # Bytes
    original_filename: str = Field(..., alias="originalFilename")

    model_config = ConfigDict(populate_by_name=True)


# Union type for all full artifact types
Artifact = Union[
    DocumentArtifact,
    SlideDeckArtifact,
    SheetArtifact,
    EconomicModelArtifact,
    StoryModelArtifact,
    ExternalDocumentArtifact,
]


class ArtifactRegistry(BaseModel):
    """Registry of all artifacts in a project."""
    documents: Dict[str, DocumentArtifact] = Field(default_factory=dict)
    slide_decks: Dict[str, SlideDeckArtifact] = Field(default_factory=dict, alias="slideDecks")
    sheets: Dict[str, SheetArtifact] = Field(default_factory=dict)
    economic_models: Dict[str, EconomicModelArtifact] = Field(default_factory=dict, alias="economicModels")
    story_models: Dict[str, StoryModelArtifact] = Field(default_factory=dict, alias="storyModels")
    external_documents: Dict[str, ExternalDocumentArtifact] = Field(default_factory=dict, alias="externalDocuments")

    model_config = ConfigDict(populate_by_name=True)


# =============================================================================
# Lightweight Reference Models (for API contracts)
# =============================================================================

class ArtifactRef(BaseModel):
    """Lightweight artifact reference for API contracts."""
    id: str = Field(..., description="Artifact ID (UUID)")
    type: ArtifactType
    title: Optional[str] = None


class DocumentRef(ArtifactRef):
    """Reference to a document artifact."""
    type: Literal[ArtifactType.document] = ArtifactType.document


class SlideDeckRef(ArtifactRef):
    """Reference to a slide deck artifact."""
    type: Literal[ArtifactType.slide_deck] = ArtifactType.slide_deck


class SheetRef(ArtifactRef):
    """Reference to a sheet artifact."""
    type: Literal[ArtifactType.sheet] = ArtifactType.sheet


# =============================================================================
# Model Registry
# =============================================================================

class ModelKind(str, Enum):
    """Types of models in the system."""
    economic = "economic"
    story = "story"


class ModelRef(BaseModel):
    """Reference to a model in the registry."""
    key: str = Field(..., description="Stable key for the model")
    kind: ModelKind
    version: Optional[str] = Field(None, description="Version or revision tag")
    config: Dict[str, Union[str, int, float, bool]] = Field(
        default_factory=dict, description="Model configuration parameters"
    )


# =============================================================================
# Media Registry
# =============================================================================

class MediaKind(str, Enum):
    """Types of media assets."""
    image = "image"
    chart = "chart"
    table = "table"
    asset = "asset"


class MediaRef(BaseModel):
    """Reference to a media asset."""
    key: str
    kind: MediaKind
    uri: str = Field(..., description="URI to the media (MinIO or external)")
    meta: Dict[str, Union[str, int, float, bool]] = Field(default_factory=dict)


# =============================================================================
# Derivation Models
# =============================================================================

class DerivationType(str, Enum):
    """Types of derivation relationships between artifacts."""
    generation = "generation"  # Target generated from source (e.g., story from econ model)
    reference = "reference"  # Target references source (e.g., chart embedded in slides)
    transformation = "transformation"  # Target is transformation of source (format conversion)
    aggregation = "aggregation"  # Target aggregates multiple sources
    filter = "filter"  # Target is filtered/subset of source
    
    # Uppercase aliases for backward compatibility
    GENERATION = "generation"
    REFERENCE = "reference"
    TRANSFORMATION = "transformation"
    AGGREGATION = "aggregation"
    FILTER = "filter"


class Derivation(BaseModel):
    """Derivation relationship between artifacts (domain model)."""
    id: str
    project_id: str = Field(..., alias="projectId")
    source_artifact_id: str = Field(..., alias="sourceArtifactId")
    target_artifact_id: str = Field(..., alias="targetArtifactId")
    derivation_type: DerivationType = Field(..., alias="derivationType")
    parameters: Dict[str, Any] = Field(default_factory=dict)  # Derivation parameters
    metadata: Dict[str, Any] = Field(default_factory=dict)  # Additional metadata
    created_at: datetime = Field(default_factory=datetime.now, alias="createdAt")
    created_by: str = Field(..., alias="createdBy")

    model_config = ConfigDict(populate_by_name=True)


class DerivationGraph(BaseModel):
    """Graph of derivation relationships in a project."""
    derivations: Dict[str, Derivation] = Field(default_factory=dict)

    def get_descendants(self, artifact_id: str) -> List[Derivation]:
        """Get all derivations where artifact_id is the source."""
        return [d for d in self.derivations.values() if d.source_artifact_id == artifact_id]

    def get_ancestors(self, artifact_id: str) -> List[Derivation]:
        """Get all derivations where artifact_id is the target."""
        return [d for d in self.derivations.values() if d.target_artifact_id == artifact_id]


# =============================================================================
# Snapshots & Instructions
# =============================================================================

class Snapshot(BaseModel):
    """Snapshot of an artifact at a point in time."""
    id: Optional[str] = None
    artifact_id: str
    label: Optional[str] = None
    data: Dict = Field(default_factory=dict)


class Instruction(BaseModel):
    """Chat instruction or operation request."""
    id: Optional[str] = None
    project_id: str
    actor: str = Field(..., description="who issued the instruction (user|system|agent)")
    text: str
    intent: Optional[str] = Field(
        None, description="Optional parsed intent label (e.g., 'update-sheet-range')"
    )
    payload: Dict = Field(default_factory=dict)


# =============================================================================
# Project Model (Lightweight)
# =============================================================================

class ProjectLight(BaseModel):
    """
    Lightweight IKAM Project model with reference-based registries.
    Used for API contracts where full artifact models aren't needed.
    """
    id: Optional[str] = Field(None, description="Project ID (UUID)")
    name: str
    description: Optional[str] = None

    artifacts: List[ArtifactRef] = Field(default_factory=list)
    models: List[ModelRef] = Field(default_factory=list)
    media: List[MediaRef] = Field(default_factory=list)

    derivations: List[Derivation] = Field(default_factory=list)

    # Fast lookup maps (optional, materialized for clients)
    artifact_index: Dict[str, ArtifactRef] = Field(default_factory=dict)
    model_index: Dict[str, ModelRef] = Field(default_factory=dict)
    media_index: Dict[str, MediaRef] = Field(default_factory=dict)

    model_config = ConfigDict(title="IKAMProject", frozen=False, json_encoders={})

    def build_indexes(self) -> None:
        """Build lookup indexes for fast access."""
        self.artifact_index = {a.id: a for a in self.artifacts}
        self.model_index = {m.key: m for m in self.models}
        self.media_index = {m.key: m for m in self.media}


class Project(BaseModel):
    """
    Complete IKAM Project model following specification v1.0.0.
    
    Represents the orchestration layer for all knowledge artifacts,
    models, and media in a venture project.
    """
    id: str
    version: str = "1.0.0"  # IKAM Project schema version (semver)
    type: Literal["project"] = "project"
    meta: ProjectMeta
    artifacts: ArtifactRegistry = Field(default_factory=ArtifactRegistry)
    derivations: DerivationGraph = Field(default_factory=DerivationGraph)

    model_config = ConfigDict(populate_by_name=True)


# =============================================================================
# Service-Layer API Models (for HTTP endpoints)
# =============================================================================

class DerivationCreate(BaseModel):
    """Input model for creating a derivation."""
    project_id: str
    source_artifact_id: str
    target_artifact_id: str
    derivation_type: DerivationType
    parameters: Dict = Field(default_factory=dict, description="Parameters used for derivation")
    metadata: Dict = Field(default_factory=dict, description="Additional metadata")


class DerivationOut(BaseModel):
    """Output model for derivation (from database)."""
    id: str
    project_id: str
    source_artifact_id: str = Field(alias="sourceArtifactId")
    target_artifact_id: str = Field(alias="targetArtifactId")
    derivation_type: str = Field(alias="derivationType")
    parameters: Dict = Field(default_factory=dict)
    metadata: Dict = Field(default_factory=dict)
    created_at: Optional[int] = Field(None, alias="createdAt")
    created_by: Optional[str] = Field(None, alias="createdBy")

    model_config = ConfigDict(populate_by_name=True)


class ImpactNode(BaseModel):
    """A node in the impact analysis graph."""
    artifact_id: str = Field(alias="artifactId")
    artifact_type: Optional[str] = Field(None, alias="artifactType")
    artifact_name: Optional[str] = Field(None, alias="artifactName")
    derivation_type: str = Field(alias="derivationType")
    depth: int = Field(..., description="Distance from source artifact")
    path: List[str] = Field(default_factory=list, description="Path of artifact IDs from source")

    model_config = ConfigDict(populate_by_name=True)


class ImpactAnalysisOut(BaseModel):
    """Output model for impact analysis."""
    source_artifact_id: str = Field(alias="sourceArtifactId")
    impacted_artifacts: List[ImpactNode] = Field(default_factory=list, alias="impactedArtifacts")
    total_impacted: int = Field(alias="totalImpacted")
    max_depth: int = Field(alias="maxDepth")

    model_config = ConfigDict(populate_by_name=True)


class RegenerationRequest(BaseModel):
    """Input model for regeneration request."""
    source_artifact_id: Optional[str] = Field(None, alias="sourceArtifactId", description="Regenerate descendants of this artifact")
    artifact_ids: Optional[List[str]] = Field(None, alias="artifactIds", description="Specific artifacts to regenerate")
    force: bool = Field(False, description="Force regeneration even if up-to-date")

    model_config = ConfigDict(populate_by_name=True)


# =============================================================================
# Helper Functions
# =============================================================================

def artifact_from_dict(data: Dict[str, Any]) -> Artifact:
    """
    Deserialize artifact from dict, dispatching to correct type based on 'type' field.
    
    Args:
        data: Artifact data dictionary with 'type' discriminator
        
    Returns:
        Typed artifact instance
        
    Raises:
        ValueError: If type is unknown or data is invalid
    """
    artifact_type = data.get("type")
    
    type_map = {
        ArtifactType.IKAM_DOCUMENT: DocumentArtifact,
        ArtifactType.IKAM_SLIDE_DECK: SlideDeckArtifact,
        ArtifactType.IKAM_SHEET: SheetArtifact,
        ArtifactType.ECONOMIC_MODEL: EconomicModelArtifact,
        ArtifactType.STORY_MODEL: StoryModelArtifact,
        ArtifactType.EXTERNAL_DOCUMENT: ExternalDocumentArtifact,
    }
    
    artifact_class = type_map.get(artifact_type)
    if not artifact_class:
        raise ValueError(f"Unknown artifact type: {artifact_type}")
    
    return artifact_class.model_validate(data)

