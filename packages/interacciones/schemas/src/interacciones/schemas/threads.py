"""Thread schemas for conversation threading.

Defines models for managing conversation threads:
- Thread: Complete thread model with all fields
- ThreadCreate: Input model for creating threads
- ThreadUpdate: Input model for updating threads
- ThreadList: Paginated list of threads with metadata
"""

from typing import Any, Dict, Optional, List
from pydantic import BaseModel, ConfigDict, Field


class ThreadBase(BaseModel):
    """Base thread model with common fields."""

    workspace_id: str = Field(..., description="Workspace ID this thread belongs to")
    project_id: Optional[str] = Field(None, description="Project ID this thread belongs to")
    title: Optional[str] = Field(None, description="Thread title (auto-generated or user-provided)")
    metadata: Dict[str, Any] = Field(
        default_factory=dict, description="Additional metadata (tags, context, etc.)"
    )

    model_config = ConfigDict(populate_by_name=True)


class ThreadCreate(ThreadBase):
    """Input model for creating a new thread."""

    model_config = ConfigDict(populate_by_name=True)


class ThreadUpdate(BaseModel):
    """Input model for updating an existing thread."""

    title: Optional[str] = Field(None, description="Updated thread title")
    metadata: Optional[Dict[str, Any]] = Field(None, description="Updated metadata")


class Thread(BaseModel):
    """Complete thread model with all fields."""

    id: str = Field(..., description="Unique thread identifier")
    workspace_id: str = Field(..., description="Workspace ID this thread belongs to")
    project_id: Optional[str] = Field(None, description="Project ID this thread belongs to")
    title: Optional[str] = Field(None, description="Thread title (auto-generated or user-provided)")
    metadata: Dict[str, Any] = Field(
        default_factory=dict, description="Additional metadata (tags, context, etc.)"
    )
    team_id: str = Field(..., description="Team owning this thread")
    created_by: str = Field(..., description="User who created the thread")
    created_at: int = Field(..., description="Thread creation timestamp (ms since epoch)")
    updated_at: int = Field(..., description="Last update timestamp (ms since epoch)")
    archived_at: Optional[int] = Field(None, description="Archive timestamp if archived")
    message_count: Optional[int] = Field(
        None, description="Number of messages in thread (computed)"
    )
    last_message_at: Optional[int] = Field(
        None, description="Timestamp of the last message in thread (ms since epoch)"
    )
    last_message_preview: Optional[str] = Field(
        None, description="Preview of last message (computed)"
    )
    head_snapshot_artifact_id: Optional[str] = Field(
        None,
        description="Current head snapshot artifact ID for this thread (mutable pointer)",
    )
    head_snapshot_updated_at: Optional[int] = Field(
        None,
        description="Timestamp when head snapshot pointer was last updated (ms since epoch)",
    )

    model_config = ConfigDict(from_attributes=True, populate_by_name=True)


class ThreadList(BaseModel):
    """Paginated list of threads."""

    threads: List[Thread] = Field(default_factory=list, description="List of threads")
    total: int = Field(..., description="Total number of threads (before pagination)")
    page: int = Field(1, ge=1, description="Current page number")
    page_size: int = Field(20, ge=1, le=100, description="Number of items per page")
    has_more: bool = Field(..., description="Whether there are more pages")


class ThreadArchive(BaseModel):
    """Response model for thread archive operation."""

    id: str = Field(..., description="Thread ID that was archived")
    archived_at: int = Field(..., description="Archive timestamp (ms since epoch)")
    message: str = Field("Thread archived successfully", description="Success message")


class ThreadHead(BaseModel):
    """Thread head snapshot pointer."""

    thread_id: str = Field(..., description="Thread ID", alias="threadId")
    head_snapshot_artifact_id: Optional[str] = Field(
        None,
        description="Current head snapshot artifact ID",
        alias="headSnapshotArtifactId",
    )
    head_snapshot_updated_at: Optional[int] = Field(
        None,
        description="Timestamp when head snapshot pointer was last updated (ms since epoch)",
        alias="headSnapshotUpdatedAt",
    )

    model_config = ConfigDict(populate_by_name=True)


class ThreadHeadUpdate(BaseModel):
    """Update request for a thread head snapshot pointer."""

    head_snapshot_artifact_id: Optional[str] = Field(
        None,
        description="Set the head snapshot artifact ID (null clears the pointer)",
        alias="headSnapshotArtifactId",
    )

    model_config = ConfigDict(populate_by_name=True)


class ThreadSnapshotMaterialization(BaseModel):
    """Result of materializing a deterministic thread snapshot."""

    thread_id: str = Field(..., description="Thread ID", alias="threadId")
    snapshot_artifact_id: str = Field(..., description="Snapshot artifact ID", alias="snapshotArtifactId")
    stream_hash: str = Field(..., description="SHA-256 of canonical snapshot bytes", alias="streamHash")
    watermark: Dict[str, Any] = Field(default_factory=dict, description="Stream watermark", alias="watermark")
    created: bool = Field(..., description="True if this call created a new artifact")

    model_config = ConfigDict(populate_by_name=True)
