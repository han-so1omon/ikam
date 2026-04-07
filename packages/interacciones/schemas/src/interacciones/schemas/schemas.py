"""Core interaction schemas for multi-agent coordination.

Defines the base types for interactions flowing through the system:
- InteractionType: Categories of interactions (user messages, agent responses, etc.)
- InteractionScopeType: Scope levels (user, agent, system)
- InteractionIn: Inbound interaction model
- InteractionOut: Outbound interaction model (includes ID, timestamp, metadata)
"""

from datetime import datetime
from enum import Enum
from typing import Any, Dict, Optional, List
from uuid import UUID, uuid4

from pydantic import BaseModel, ConfigDict, Field


class InteractionType(str, Enum):
    """Types of interactions flowing through the system."""

    USER_MESSAGE = "user_message"
    """User sends a message to the assistant."""

    ASSISTANT_RESPONSE = "assistant_response"
    """Assistant responds to the user."""

    AGENT_REQUEST = "agent_request"
    """System requests an agent to perform work."""

    AGENT_RESPONSE = "agent_response"
    """Agent responds with results."""

    SYSTEM_EVENT = "system_event"
    """System-generated event (errors, status updates, etc.)."""


class InteractionScopeType(str, Enum):
    """Scope levels for interactions."""

    USER = "user"
    """User-scoped interaction (tied to a specific user session)."""

    AGENT = "agent"
    """Agent-scoped interaction (tied to a specific agent)."""

    SYSTEM = "system"
    """System-scoped interaction (global, not tied to user or agent)."""


class InteractionIn(BaseModel):
    """Inbound interaction model (client → server).

    Represents an interaction being sent into the system. Does not include
    server-generated fields like ID or timestamp.

    Example:
        >>> interaction = InteractionIn(
        ...     type=InteractionType.USER_MESSAGE,
        ...     scope="user",
        ...     content="Analyze Q3 sales data",
        ...     metadata={"user_id": "123", "session_id": "abc"}
        ... )
    """

    type: InteractionType
    """Type of interaction (user_message, agent_response, etc.)."""

    scope: InteractionScopeType
    """Scope level (user, agent, system)."""

    content: str
    """Main interaction content (message text, agent payload, etc.)."""

    metadata: Dict[str, Any] = Field(default_factory=dict)
    """Additional context (user_id, session_id, agent_id, etc.)."""

    parent_id: Optional[str] = None
    """Optional parent interaction ID (database SERIAL as string) for threading."""


class InteractionOut(BaseModel):
    """Outbound interaction model (server → client).

    Represents an interaction stored in the system. Includes server-generated
    fields like ID, timestamp, and processing metadata.

    Example:
        >>> interaction = InteractionOut(
        ...     id="123",
        ...     type=InteractionType.AGENT_RESPONSE,
        ...     scope="agent",
        ...     content=json.dumps({"status": "complete"}),
        ...     metadata={"agent_id": "econ-modeler"},
        ...     created_at=datetime.utcnow()
        ... )
    """

    id: str
    """Unique interaction ID (database SERIAL as string)."""

    type: InteractionType
    """Type of interaction (user_message, agent_response, etc.)."""

    scope: InteractionScopeType
    """Scope level (user, agent, system)."""

    content: str
    """Main interaction content (message text, agent payload, etc.)."""

    metadata: Dict[str, Any] = Field(default_factory=dict)
    """Additional context (user_id, session_id, agent_id, etc.)."""

    parent_id: Optional[str] = None
    """Optional parent interaction ID for threading."""

    created_at: int
    """Unix timestamp (milliseconds) when the interaction was created."""

    processed_at: Optional[int] = None
    """Unix timestamp (milliseconds) when the interaction was processed (for async workflows)."""

    model_config = ConfigDict(
        json_schema_extra={
            "example": {
                "id": "550e8400-e29b-41d4-a716-446655440000",
                "type": "agent_response",
                "scope": "agent",
                "content": '{"status": "complete", "result": {...}}',
                "metadata": {"agent_id": "econ-modeler", "duration_ms": 1250},
                "parent_id": "550e8400-e29b-41d4-a716-446655440001",
                "created_at": "2025-10-29T12:00:00Z",
                "processed_at": "2025-10-29T12:00:01.25Z",
            }
        }
    )


# --- Unified Response Envelope --------------------------------------------

class ArtifactDescriptor(BaseModel):
    """Descriptor for a renderable IKAM artifact returned to the frontend.

    Either `url` (persisted artifact) or `data` (inline transient artifact) must
    be provided. Inline data enables UI rendering when backend persistence is
    not yet available (e.g., service-to-service auth still pending).
    """

    id: str
    type: str  # e.g., "ikam:SlideDeck", "ikam:Workbook", "ikam:Document"
    title: str
    url: Optional[str] = None  # Backend fetch endpoint (/api/model/artifacts/{id})
    data: Optional[Dict[str, Any]] = None  # Inline IKAM artifact structure


class ChatMessage(BaseModel):
    """Simple chat message payload for user-visible text responses."""

    role: str = Field(default="assistant")
    text: str
    markdown: bool = Field(default=True)


class InteractionResponse(BaseModel):
    """Unified envelope combining chat and render entries for the UI.

    - chat: messages to display in the chat panel
    - render: IKAM artifacts to open in tabs in the interactions area
    """

    summary: Optional[str] = None
    chat: List[ChatMessage] = Field(default_factory=list)
    render: List[ArtifactDescriptor] = Field(default_factory=list)

