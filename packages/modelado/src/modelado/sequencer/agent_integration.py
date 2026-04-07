"""Integration of MCP sequencer tools with agent registry.

Provides a unified interface for registering sequencer tools as an MCP agent
and initializing all required tool handlers.
"""

import logging
import os
from typing import Optional, Dict, Any

logger = logging.getLogger(__name__)


try:
    from narraciones_mcp_base.agent_registry import register_sequencer_agent as _register_sequencer_agent
except ImportError:  # pragma: no cover
    _register_sequencer_agent = None


def initialize_sequencer_agent(
    agent_id: str = "mcp-sequencer",
    base_api_url: Optional[str] = None,
    service_url: Optional[str] = None,
) -> Dict[str, Any]:
    """Initialize and register sequencer agent with MCP tools.
    
    Performs these steps:
    1. Load environment configuration
    2. Register agent with base API registry (with retry)
    3. Verify database connectivity (optional)
    4. Return agent status and tool metadata
    
    Args:
        agent_id: Agent identifier
        base_api_url: Base API URL (defaults to env BASE_API_URL or http://localhost:8000)
        service_url: Service URL (defaults to env MCP_SERVICE_URL)
    
    Returns:
        Dict with initialization status, agent info, and tools metadata
    """
    from modelado.sequencer.mcp_tools import MCP_TOOLS
    
    # Load environment configuration
    base_api_url = base_api_url or os.getenv("BASE_API_URL", "http://localhost:8000")
    service_url = service_url or os.getenv("MCP_SERVICE_URL")
    
    # Attempt agent registration
    registration_success = False
    registration_error = None
    
    if _register_sequencer_agent is None:
        registration_success = False
        registration_error = (
            "narraciones_mcp_base not available; skipping agent registration"
        )
        logger.info(registration_error)
    else:
        try:
            logger.info(f"Registering sequencer agent {agent_id}")
            registration_success = _register_sequencer_agent(
                agent_id=agent_id,
                base_api_url=base_api_url,
                url=service_url,
                timeout=5.0,
            )
        except Exception as e:
            logger.warning(f"Agent registration failed: {str(e)}")
            registration_error = str(e)
    
    # Compile tool metadata
    tools_metadata = {}
    for tool_name, tool_spec in MCP_TOOLS.items():
        tools_metadata[tool_name] = {
            "name": tool_spec.get("name"),
            "description": tool_spec.get("description"),
            "input_schema": tool_spec.get("inputSchema", {}),
        }
    
    return {
        "status": "ready" if registration_success else "ready_unregistered",
        "agent_id": agent_id,
        "base_api_url": base_api_url,
        "service_url": service_url,
        "registration": {
            "success": registration_success,
            "error": registration_error,
        },
        "tools": tools_metadata,
        "capabilities": {
            "domains": ["planning", "project-management", "estimation"],
            "actions": ["create_sequence", "validate_sequence", "commit_sequence"],
            "tags": ["sequencer", "ikam-aware", "provenance-tracking"],
        },
    }


def get_sequencer_tools() -> Dict[str, Any]:
    """Get all registered sequencer MCP tools.
    
    Returns:
        Dict mapping tool names to tool specifications
    """
    from modelado.sequencer.mcp_tools import MCP_TOOLS
    return MCP_TOOLS


def get_tool_handler(tool_name: str):
    """Get the handler function for a specific tool.
    
    Args:
        tool_name: Name of the tool (create_sequence, validate_sequence, commit_sequence)
    
    Returns:
        Handler function for the tool
    
    Raises:
        ValueError: If tool_name is not recognized
    """
    from modelado.sequencer.mcp_tools import (
        create_sequence,
        validate_sequence_tool,
        commit_sequence,
    )
    
    handlers = {
        "create_sequence": create_sequence,
        "validate_sequence": validate_sequence_tool,
        "commit_sequence": commit_sequence,
    }
    
    if tool_name not in handlers:
        raise ValueError(f"Unknown tool: {tool_name}. Available: {list(handlers.keys())}")
    
    return handlers[tool_name]
