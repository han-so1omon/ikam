"""Tests for agent registry integration with MCP sequencer tools."""

import pytest
from unittest.mock import Mock, patch, MagicMock
from modelado.sequencer.agent_integration import (
    initialize_sequencer_agent,
    get_sequencer_tools,
    get_tool_handler,
)


class TestAgentRegistration:
    """Test agent registry registration."""

    def test_register_sequencer_agent_success(self):
        """Test successful agent registration."""
        with patch("modelado.sequencer.agent_integration._register_sequencer_agent") as mock_register:
            mock_register.return_value = True
            
            result = initialize_sequencer_agent(
                agent_id="test-sequencer",
                base_api_url="http://test-api:8000",
            )
            
            assert result["status"] == "ready"
            assert result["registration"]["success"] is True
            assert result["agent_id"] == "test-sequencer"
            assert result["base_api_url"] == "http://test-api:8000"
            
            mock_register.assert_called_once()

    def test_register_sequencer_agent_failure_continues(self):
        """Test that registration failure doesn't block initialization."""
        with patch("modelado.sequencer.agent_integration._register_sequencer_agent") as mock_register:
            mock_register.return_value = False
            
            result = initialize_sequencer_agent(
                agent_id="test-sequencer",
            )
            
            # Should still return ready but mark as unregistered
            assert result["status"] == "ready_unregistered"
            assert result["registration"]["success"] is False

    def test_register_sequencer_agent_exception_handling(self):
        """Test that registration exceptions are caught and logged."""
        with patch("modelado.sequencer.agent_integration._register_sequencer_agent") as mock_register:
            mock_register.side_effect = Exception("Connection refused")
            
            result = initialize_sequencer_agent(
                agent_id="test-sequencer",
            )
            
            assert result["status"] == "ready_unregistered"
            assert result["registration"]["success"] is False
            assert "Connection refused" in result["registration"]["error"]

    def test_agent_initialization_includes_tools_metadata(self):
        """Test that initialization includes MCP tools metadata."""
        with patch("modelado.sequencer.agent_integration._register_sequencer_agent") as mock_register:
            mock_register.return_value = True
            
            result = initialize_sequencer_agent()
            
            assert "tools" in result
            assert "create_sequence" in result["tools"]
            assert "validate_sequence" in result["tools"]
            assert "commit_sequence" in result["tools"]
            
            # Verify each tool has required metadata
            for tool_name, tool_meta in result["tools"].items():
                assert "name" in tool_meta
                assert "description" in tool_meta
                assert "input_schema" in tool_meta

    def test_agent_initialization_includes_capabilities(self):
        """Test that initialization includes capability metadata."""
        with patch("modelado.sequencer.agent_integration._register_sequencer_agent") as mock_register:
            mock_register.return_value = True
            
            result = initialize_sequencer_agent()
            
            assert "capabilities" in result
            assert "domains" in result["capabilities"]
            assert "actions" in result["capabilities"]
            assert "tags" in result["capabilities"]
            
            assert "planning" in result["capabilities"]["domains"]
            assert "create_sequence" in result["capabilities"]["actions"]
            assert "ikam-aware" in result["capabilities"]["tags"]


class TestToolHandlers:
    """Test tool handler retrieval and execution."""

    def test_get_sequencer_tools_returns_all_tools(self):
        """Test that all sequencer tools are available."""
        tools = get_sequencer_tools()
        
        assert "create_sequence" in tools
        assert "validate_sequence" in tools
        assert "commit_sequence" in tools
        
        # Verify each tool has required fields
        for tool_name, tool_spec in tools.items():
            assert "name" in tool_spec
            assert "description" in tool_spec
            assert "inputSchema" in tool_spec

    def test_get_tool_handler_create_sequence(self):
        """Test retrieving create_sequence handler."""
        handler = get_tool_handler("create_sequence")
        
        # Should be callable
        assert callable(handler)
        
        # Verify it's the correct function
        assert handler.__name__ == "create_sequence"

    def test_get_tool_handler_validate_sequence(self):
        """Test retrieving validate_sequence handler."""
        handler = get_tool_handler("validate_sequence")
        
        assert callable(handler)
        assert handler.__name__ == "validate_sequence_tool"

    def test_get_tool_handler_commit_sequence(self):
        """Test retrieving commit_sequence handler."""
        handler = get_tool_handler("commit_sequence")
        
        assert callable(handler)
        assert handler.__name__ == "commit_sequence"

    def test_get_tool_handler_unknown_tool_raises_error(self):
        """Test that requesting unknown tool raises ValueError."""
        with pytest.raises(ValueError) as exc_info:
            get_tool_handler("unknown_tool")
        
        assert "Unknown tool" in str(exc_info.value)
        assert "unknown_tool" in str(exc_info.value)

    def test_get_tool_handler_available_tools_in_error_message(self):
        """Test that error message includes available tools."""
        with pytest.raises(ValueError) as exc_info:
            get_tool_handler("invalid")
        
        error_msg = str(exc_info.value)
        assert "create_sequence" in error_msg
        assert "validate_sequence" in error_msg
        assert "commit_sequence" in error_msg


class TestAgentEnvironmentConfiguration:
    """Test environment variable configuration."""

    def test_initialization_respects_env_base_api_url(self):
        """Test that BASE_API_URL environment variable is respected."""
        import os
        with patch.dict(os.environ, {"BASE_API_URL": "http://custom-api:9000"}):
            with patch("modelado.sequencer.agent_integration._register_sequencer_agent"):
                result = initialize_sequencer_agent()
                
                assert result["base_api_url"] == "http://custom-api:9000"

    def test_initialization_respects_env_service_url(self):
        """Test that MCP_SERVICE_URL environment variable is respected."""
        import os
        with patch.dict(os.environ, {"MCP_SERVICE_URL": "http://custom-service:8080"}):
            with patch("modelado.sequencer.agent_integration._register_sequencer_agent"):
                result = initialize_sequencer_agent()
                
                assert result["service_url"] == "http://custom-service:8080"

    def test_initialization_parameter_overrides_env(self):
        """Test that explicit parameters override environment variables."""
        import os
        with patch.dict(os.environ, {"BASE_API_URL": "http://env-api:8000"}):
            with patch("modelado.sequencer.agent_integration._register_sequencer_agent"):
                result = initialize_sequencer_agent(
                    base_api_url="http://explicit-api:9000"
                )
                
                assert result["base_api_url"] == "http://explicit-api:9000"

    def test_initialization_uses_defaults_without_env(self):
        """Test that sensible defaults are used without environment variables."""
        import os
        env = os.environ.copy()
        env.pop("BASE_API_URL", None)
        env.pop("MCP_SERVICE_URL", None)
        
        with patch.dict(os.environ, env, clear=True):
            with patch("modelado.sequencer.agent_integration._register_sequencer_agent"):
                result = initialize_sequencer_agent()
                
                # Should use default localhost
                assert "localhost:8000" in result["base_api_url"] or "http://" in result["base_api_url"]
