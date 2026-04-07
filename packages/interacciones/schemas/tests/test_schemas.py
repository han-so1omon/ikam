"""Unit tests for interaction schemas.

Tests the core Pydantic models (InteractionIn, InteractionOut) and enums
(InteractionType, InteractionScopeType).
"""

import pytest
from datetime import datetime
from uuid import UUID, uuid4

from interacciones.schemas import (
    InteractionType,
    InteractionScopeType,
    InteractionIn,
    InteractionOut,
)


class TestInteractionType:
    """Tests for InteractionType enum."""

    def test_enum_values(self):
        """Test that all expected enum values exist."""
        assert InteractionType.USER_MESSAGE == "user_message"
        assert InteractionType.ASSISTANT_RESPONSE == "assistant_response"
        assert InteractionType.AGENT_REQUEST == "agent_request"
        assert InteractionType.AGENT_RESPONSE == "agent_response"
        assert InteractionType.SYSTEM_EVENT == "system_event"

    def test_enum_from_string(self):
        """Test that enum values can be constructed from strings."""
        assert InteractionType("user_message") == InteractionType.USER_MESSAGE
        assert InteractionType("agent_response") == InteractionType.AGENT_RESPONSE


class TestInteractionScopeType:
    """Tests for InteractionScopeType enum."""

    def test_enum_values(self):
        """Test that all expected enum values exist."""
        assert InteractionScopeType.USER == "user"
        assert InteractionScopeType.AGENT == "agent"
        assert InteractionScopeType.SYSTEM == "system"

    def test_enum_from_string(self):
        """Test that enum values can be constructed from strings."""
        assert InteractionScopeType("user") == InteractionScopeType.USER
        assert InteractionScopeType("agent") == InteractionScopeType.AGENT


class TestInteractionIn:
    """Tests for InteractionIn model."""

    def test_minimal_interaction(self):
        """Test creating an interaction with only required fields."""
        interaction = InteractionIn(
            type=InteractionType.USER_MESSAGE,
            scope=InteractionScopeType.USER,
            content="Hello world",
        )

        assert interaction.type == InteractionType.USER_MESSAGE
        assert interaction.scope == InteractionScopeType.USER
        assert interaction.content == "Hello world"
        assert interaction.metadata == {}
        assert interaction.parent_id is None

    def test_full_interaction(self):
        """Test creating an interaction with all fields."""
        parent_id = "123"  # Database SERIAL ID as string
        interaction = InteractionIn(
            type=InteractionType.AGENT_RESPONSE,
            scope=InteractionScopeType.AGENT,
            content='{"status": "complete"}',
            metadata={"agent_id": "econ-modeler", "duration_ms": 1250},
            parent_id=parent_id,
        )

        assert interaction.type == InteractionType.AGENT_RESPONSE
        assert interaction.scope == InteractionScopeType.AGENT
        assert interaction.content == '{"status": "complete"}'
        assert interaction.metadata == {"agent_id": "econ-modeler", "duration_ms": 1250}
        assert interaction.parent_id == parent_id

    def test_serialization(self):
        """Test that the model can be serialized to JSON."""
        interaction = InteractionIn(
            type=InteractionType.USER_MESSAGE,
            scope=InteractionScopeType.USER,
            content="Test message",
            metadata={"user_id": "123"},
        )

        json_data = interaction.model_dump(mode="json")
        assert json_data["type"] == "user_message"
        assert json_data["scope"] == "user"
        assert json_data["content"] == "Test message"
        assert json_data["metadata"] == {"user_id": "123"}

    def test_deserialization(self):
        """Test that the model can be deserialized from JSON."""
        json_data = {
            "type": "agent_request",
            "scope": "agent",
            "content": "Process data",
            "metadata": {"agent_id": "story-modeler"},
        }

        interaction = InteractionIn(**json_data)
        assert interaction.type == InteractionType.AGENT_REQUEST
        assert interaction.scope == InteractionScopeType.AGENT
        assert interaction.content == "Process data"
        assert interaction.metadata == {"agent_id": "story-modeler"}


class TestInteractionOut:
    """Tests for InteractionOut model."""

    def test_minimal_interaction(self):
        """Test creating an interaction with only required fields."""
        interaction = InteractionOut(
            id="1",
            type=InteractionType.USER_MESSAGE,
            scope=InteractionScopeType.USER,
            content="Hello world",
            created_at=1699660800000,
        )

        # Server-generated fields
        assert interaction.id == "1"
        assert interaction.created_at == 1699660800000
        assert interaction.processed_at is None

        # User-provided fields
        assert interaction.type == InteractionType.USER_MESSAGE
        assert interaction.scope == InteractionScopeType.USER
        assert interaction.content == "Hello world"
        assert interaction.metadata == {}
        assert interaction.parent_id is None

    def test_full_interaction(self):
        """Test creating an interaction with all fields."""
        interaction_id = "42"
        parent_id = "123"
        created_at = 1699660800000  # Unix timestamp ms
        processed_at = 1699660801000  # Unix timestamp ms

        interaction = InteractionOut(
            id=interaction_id,
            type=InteractionType.AGENT_RESPONSE,
            scope=InteractionScopeType.AGENT,
            content='{"status": "complete"}',
            metadata={"agent_id": "econ-modeler"},
            parent_id=parent_id,
            created_at=created_at,
            processed_at=processed_at,
        )

        assert interaction.id == interaction_id
        assert interaction.type == InteractionType.AGENT_RESPONSE
        assert interaction.scope == InteractionScopeType.AGENT
        assert interaction.content == '{"status": "complete"}'
        assert interaction.metadata == {"agent_id": "econ-modeler"}
        assert interaction.parent_id == parent_id
        assert interaction.created_at == created_at
        assert interaction.processed_at == processed_at

    def test_serialization(self):
        """Test that the model can be serialized to JSON."""
        interaction = InteractionOut(
            id="42",
            type=InteractionType.USER_MESSAGE,
            scope=InteractionScopeType.USER,
            content="Test message",
            metadata={"user_id": "123"},
            created_at=1699660800000,
        )

        json_data = interaction.model_dump(mode="json")
        assert json_data["id"] == "42"
        assert json_data["type"] == "user_message"
        assert json_data["scope"] == "user"
        assert json_data["content"] == "Test message"
        assert json_data["metadata"] == {"user_id": "123"}
        assert json_data["created_at"] == 1699660800000

    def test_deserialization(self):
        """Test that the model can be deserialized from JSON."""
        json_data = {
            "id": "100",
            "type": "agent_response",
            "scope": "agent",
            "content": '{"status": "complete"}',
            "metadata": {"agent_id": "econ-modeler"},
            "parent_id": "99",
            "created_at": 1699660800000,
            "processed_at": 1699660801000,
        }

        interaction = InteractionOut(**json_data)
        assert interaction.id == "100"
        assert interaction.type == InteractionType.AGENT_RESPONSE
        assert interaction.scope == InteractionScopeType.AGENT
        assert interaction.content == '{"status": "complete"}'
        assert interaction.parent_id == "99"
        assert interaction.created_at == 1699660800000
        assert interaction.processed_at == 1699660801000

