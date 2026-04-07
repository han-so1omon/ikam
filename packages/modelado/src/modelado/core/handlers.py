"""Handler protocols for generative modeling services.

REDESIGNED FOR COMPLETE GENERATIVITY:
- Handlers interpret semantic commands (no action enums)
- SemanticEngine generates operations on-demand
- All handlers are semantic interpreters, not action routers
"""

from __future__ import annotations

from typing import Protocol

from .commands import (
    SemanticEconomicCommand,
    SemanticEconomicResult,
    SemanticStoryCommand,
    SemanticStoryResult,
)


class SemanticEconomicHandler(Protocol):
    """Handler for semantic economic commands.
    
    Takes a natural language instruction + context,
    generates and executes the appropriate economic operation,
    returns result with generation provenance.
    """

    async def handle(self, command: SemanticEconomicCommand) -> SemanticEconomicResult:
        """Execute semantic economic instruction.
        
        Args:
            command: SemanticEconomicCommand with instruction + context
            
        Returns:
            SemanticEconomicResult with payload and generation metadata
        """
        ...


class SemanticStoryHandler(Protocol):
    """Handler for semantic story commands.
    
    Takes a natural language instruction + context,
    generates and executes the appropriate story operation,
    returns result with generation provenance.
    """

    async def handle(self, command: SemanticStoryCommand) -> SemanticStoryResult:
        """Execute semantic story instruction.
        
        Args:
            command: SemanticStoryCommand with instruction + context
            
        Returns:
            SemanticStoryResult with payload and generation metadata
        """
        ...

