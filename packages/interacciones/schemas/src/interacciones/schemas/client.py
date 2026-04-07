"""HTTP client for interactions API.

Provides a lightweight client for sending interactions to the base API and
retrieving interaction history.

Example:
    >>> client = InteractionsClient(base_url="http://localhost:8000")
    >>> interaction = await client.send(
    ...     type=InteractionType.USER_MESSAGE,
    ...     scope="user",
    ...     content="Analyze sales data",
    ...     metadata={"user_id": "123"}
    ... )
    >>> print(interaction.id)
"""

import httpx
from typing import Dict, Any, Optional, List

from .schemas import InteractionType, InteractionScopeType, InteractionIn, InteractionOut


class InteractionsClient:
    """HTTP client for interactions API.

    Args:
        base_url: Base URL of the interactions API (e.g., "http://localhost:8000")
        timeout: Request timeout in seconds (default: 30)
    """

    def __init__(self, base_url: str, timeout: float = 30.0):
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout
        self._client: Optional[httpx.AsyncClient] = None

    async def __aenter__(self) -> "InteractionsClient":
        """Async context manager entry."""
        self._client = httpx.AsyncClient(
            base_url=self.base_url,
            timeout=self.timeout,
        )
        return self

    async def __aexit__(self, exc_type, exc_val, exc_tb) -> None:
        """Async context manager exit."""
        if self._client:
            await self._client.aclose()

    async def send(
        self,
        type: InteractionType,
        scope: InteractionScopeType,
        content: str,
        metadata: Optional[Dict[str, Any]] = None,
        parent_id: Optional[str] = None,
    ) -> InteractionOut:
        """Send an interaction to the API.

        Args:
            type: Type of interaction (user_message, agent_response, etc.)
            scope: Scope level (user, agent, system)
            content: Main interaction content
            metadata: Optional additional context
            parent_id: Optional parent interaction ID (as string) for threading

        Returns:
            The created interaction with server-generated fields (ID, timestamp, etc.)

        Raises:
            httpx.HTTPStatusError: If the API returns an error status
        """
        if not self._client:
            raise RuntimeError("Client not initialized. Use `async with` context manager.")

        interaction_in = InteractionIn(
            type=type,
            scope=scope,
            content=content,
            metadata=metadata or {},
            parent_id=parent_id,
        )

        response = await self._client.post(
            "/api/interactions",
            json=interaction_in.model_dump(mode="json"),
        )
        response.raise_for_status()

        return InteractionOut(**response.json())

    async def get(self, interaction_id: str) -> InteractionOut:
        """Get a single interaction by ID.

        Args:
            interaction_id: Unique interaction ID (as string)

        Returns:
            The requested interaction

        Raises:
            httpx.HTTPStatusError: If the interaction doesn't exist or API returns error
        """
        if not self._client:
            raise RuntimeError("Client not initialized. Use `async with` context manager.")

        response = await self._client.get(f"/api/interactions/{interaction_id}")
        response.raise_for_status()

        return InteractionOut(**response.json())

    async def list(
        self,
        scope: Optional[InteractionScopeType] = None,
        type: Optional[InteractionType] = None,
        parent_id: Optional[str] = None,
        limit: int = 100,
        offset: int = 0,
    ) -> List[InteractionOut]:
        """List interactions with optional filtering.

        Args:
            scope: Filter by scope level (user, agent, system)
            type: Filter by interaction type
            parent_id: Filter by parent interaction ID (as string)
            limit: Maximum number of results (default: 100)
            offset: Offset for pagination (default: 0)

        Returns:
            List of matching interactions

        Raises:
            httpx.HTTPStatusError: If API returns error
        """
        if not self._client:
            raise RuntimeError("Client not initialized. Use `async with` context manager.")

        params: Dict[str, Any] = {"limit": limit, "offset": offset}
        if scope:
            params["scope"] = scope.value
        if type:
            params["type"] = type.value
        if parent_id:
            params["parent_id"] = parent_id

        response = await self._client.get("/api/interactions", params=params)
        response.raise_for_status()

        return [InteractionOut(**item) for item in response.json()]
