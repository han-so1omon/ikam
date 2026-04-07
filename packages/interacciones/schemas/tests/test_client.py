"""Unit tests for InteractionsClient.

Tests the HTTP client for interactions API using httpx mocks.
"""

import pytest

import httpx
from respx import MockRouter

from interacciones.schemas import (
    InteractionType,
    InteractionScopeType,
    InteractionsClient,
    InteractionOut,
)


@pytest.fixture
def base_url() -> str:
    """Base URL for test client."""
    return "http://test-api:8000"


@pytest.fixture
def client(base_url: str) -> InteractionsClient:
    """Create test client."""
    return InteractionsClient(base_url=base_url)


@pytest.mark.asyncio
class TestInteractionsClient:
    """Tests for InteractionsClient."""

    async def test_context_manager(self, client: InteractionsClient):
        """Test that client works as async context manager."""
        async with client as c:
            assert c._client is not None
            assert isinstance(c._client, httpx.AsyncClient)
        # After exit, client should be closed
        # (we can't test this directly without accessing private state)

    async def test_send_interaction(self, client: InteractionsClient, base_url: str):
        """Test sending an interaction."""
        interaction_id = "42"
        created_at = 1699660800000  # Unix timestamp in milliseconds

        async with client:
            with MockRouter() as respx_mock:
                # Mock the POST /api/interactions endpoint
                respx_mock.post(f"{base_url}/api/interactions").mock(
                    return_value=httpx.Response(
                        status_code=200,
                        json={
                            "id": interaction_id,
                            "type": "user_message",
                            "scope": "user",
                            "content": "Test message",
                            "metadata": {"user_id": "123"},
                            "parent_id": None,
                            "created_at": created_at,
                            "processed_at": None,
                        },
                    )
                )

                result = await client.send(
                    type=InteractionType.USER_MESSAGE,
                    scope=InteractionScopeType.USER,
                    content="Test message",
                    metadata={"user_id": "123"},
                )

                assert isinstance(result, InteractionOut)
                assert result.id == interaction_id
                assert result.type == InteractionType.USER_MESSAGE
                assert result.scope == InteractionScopeType.USER
                assert result.content == "Test message"
                assert result.metadata == {"user_id": "123"}

    async def test_send_interaction_with_parent(
        self, client: InteractionsClient, base_url: str
    ):
        """Test sending an interaction with parent_id."""
        interaction_id = "100"
        parent_id = "99"

        async with client:
            with MockRouter() as respx_mock:
                respx_mock.post(f"{base_url}/api/interactions").mock(
                    return_value=httpx.Response(
                        status_code=200,
                        json={
                            "id": interaction_id,
                            "type": "assistant_response",
                            "scope": "user",
                            "content": "Response",
                            "metadata": {},
                            "parent_id": parent_id,
                            "created_at": 1699660800000,
                            "processed_at": None,
                        },
                    )
                )

                result = await client.send(
                    type=InteractionType.ASSISTANT_RESPONSE,
                    scope=InteractionScopeType.USER,
                    content="Response",
                    parent_id=parent_id,
                )

                assert result.parent_id == parent_id

    async def test_get_interaction(self, client: InteractionsClient, base_url: str):
        """Test getting a single interaction by ID."""
        interaction_id = "123"

        async with client:
            with MockRouter() as respx_mock:
                respx_mock.get(f"{base_url}/api/interactions/{interaction_id}").mock(
                    return_value=httpx.Response(
                        status_code=200,
                        json={
                            "id": interaction_id,
                            "type": "agent_response",
                            "scope": "agent",
                            "content": '{"status": "complete"}',
                            "metadata": {"agent_id": "econ-modeler"},
                            "parent_id": None,
                            "created_at": 1699660800000,
                            "processed_at": 1699660801000,
                        },
                    )
                )

                result = await client.get(interaction_id)

                assert isinstance(result, InteractionOut)
                assert result.id == interaction_id
                assert result.type == InteractionType.AGENT_RESPONSE
                assert result.metadata["agent_id"] == "econ-modeler"

    async def test_list_interactions(self, client: InteractionsClient, base_url: str):
        """Test listing interactions with filters."""
        interaction1_id = "200"
        interaction2_id = "201"

        async with client:
            with MockRouter() as respx_mock:
                respx_mock.get(f"{base_url}/api/interactions").mock(
                    return_value=httpx.Response(
                        status_code=200,
                        json=[
                            {
                                "id": interaction1_id,
                                "type": "user_message",
                                "scope": "user",
                                "content": "Message 1",
                                "metadata": {},
                                "parent_id": None,
                                "created_at": 1699660800000,
                                "processed_at": None,
                            },
                            {
                                "id": interaction2_id,
                                "type": "assistant_response",
                                "scope": "user",
                                "content": "Response 1",
                                "metadata": {},
                                "parent_id": interaction1_id,
                                "created_at": 1699660801000,
                                "processed_at": None,
                            },
                        ],
                    )
                )

                results = await client.list(
                    scope=InteractionScopeType.USER,
                    limit=10,
                    offset=0,
                )

                assert len(results) == 2
                assert all(isinstance(r, InteractionOut) for r in results)
                assert results[0].id == interaction1_id
                assert results[1].id == interaction2_id
                assert results[1].parent_id == interaction1_id

    async def test_list_with_filters(self, client: InteractionsClient, base_url: str):
        """Test listing interactions with type and parent_id filters."""
        parent_id = "500"

        async with client:
            with MockRouter() as respx_mock:
                route = respx_mock.get(f"{base_url}/api/interactions").mock(
                    return_value=httpx.Response(status_code=200, json=[])
                )

                await client.list(
                    scope=InteractionScopeType.AGENT,
                    type=InteractionType.AGENT_RESPONSE,
                    parent_id=parent_id,
                    limit=50,
                    offset=10,
                )

                # Verify query parameters were sent correctly
                request = route.calls[0].request
                assert request.url.params["scope"] == "agent"
                assert request.url.params["type"] == "agent_response"
                assert request.url.params["parent_id"] == parent_id
                assert request.url.params["limit"] == "50"
                assert request.url.params["offset"] == "10"

    async def test_send_without_context_manager(self, client: InteractionsClient):
        """Test that send raises error when used outside context manager."""
        with pytest.raises(RuntimeError, match="Client not initialized"):
            await client.send(
                type=InteractionType.USER_MESSAGE,
                scope=InteractionScopeType.USER,
                content="Test",
            )

    async def test_get_without_context_manager(self, client: InteractionsClient):
        """Test that get raises error when used outside context manager."""
        with pytest.raises(RuntimeError, match="Client not initialized"):
            await client.get("123")

    async def test_list_without_context_manager(self, client: InteractionsClient):
        """Test that list raises error when used outside context manager."""
        with pytest.raises(RuntimeError, match="Client not initialized"):
            await client.list()

    async def test_http_error_handling(self, client: InteractionsClient, base_url: str):
        """Test that HTTP errors are raised properly."""
        async with client:
            with MockRouter() as respx_mock:
                respx_mock.post(f"{base_url}/api/interactions").mock(
                    return_value=httpx.Response(
                        status_code=422,
                        json={"detail": "Validation error"},
                    )
                )

                with pytest.raises(httpx.HTTPStatusError):
                    await client.send(
                        type=InteractionType.USER_MESSAGE,
                        scope=InteractionScopeType.USER,
                        content="Test",
                    )
