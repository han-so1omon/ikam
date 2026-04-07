"""Shared modeling command contracts and utilities for Narraciones services.

REDESIGNED FOR COMPLETE GENERATIVITY:
- Semantic command contracts (SemanticEconomicCommand, SemanticStoryCommand)
- No action enums (EconomicAction, StoryAction removed)
- All operations generated on-demand via SemanticEngine

⚠️ COMPATIBILITY NOTE:
EconomicAction and StoryAction are available from .compat for backward compatibility
with test infrastructure only. DO NOT use in new code. Use semantic commands instead.
See: docs/sprints/PHASE8_TASK7_STATUS.md
"""

from .commands import (
    SemanticEconomicCommand,
    SemanticEconomicResult,
    SemanticStoryCommand,
    SemanticStoryResult,
    EconomicStatus,
    StoryStatus,
    ModelingCommand,
    ModelingResult,
    requires_immediate_reply,
)
from .handlers import SemanticEconomicHandler, SemanticStoryHandler
# Compatibility shim for deprecated enum-based commands (test infrastructure only)
from .compat import (  # noqa: F401 (imported but unused - for tests)
    EconomicAction,
    StoryAction,
    EconomicCommand,
    EconomicResult,
    StoryCommand,
    StoryResult,
    ModelingTopicConfig,
    ModelingCommandCodec,
)
from .adapters import (
    ModelingAdapters,
    EconomicPersistenceAdapter,
    StoryPersistenceAdapter,
    PendingChangesAdapter,
    ProjectMetaAdapter,
    TelemetryAdapter,
    get_modeling_adapters,
    register_modeling_adapters,
)

__all__ = [
    # Semantic command contracts
    "SemanticEconomicCommand",
    "SemanticEconomicResult",
    "SemanticEconomicHandler",
    "SemanticStoryCommand",
    "SemanticStoryResult",
    "SemanticStoryHandler",
    # Status enums
    "EconomicStatus",
    "StoryStatus",
    # Type aliases
    "ModelingCommand",
    "ModelingResult",
    # Utilities
    "requires_immediate_reply",
    # Deprecated (for test compatibility only)
    "EconomicAction",       # ⚠️ Use SemanticEconomicCommand instead
    "StoryAction",          # ⚠️ Use SemanticStoryCommand instead
    "EconomicCommand",      # ⚠️ Use SemanticEconomicCommand instead
    "EconomicResult",       # ⚠️ Use SemanticEconomicResult instead
    "StoryCommand",         # ⚠️ Use SemanticStoryCommand instead
    "StoryResult",          # ⚠️ Use SemanticStoryResult instead
    "ModelingTopicConfig",  # ⚠️ Deprecated Kafka infrastructure
    "ModelingCommandCodec", # ⚠️ Deprecated Kafka infrastructure
    # Adapters
    "ModelingAdapters",
    "EconomicPersistenceAdapter",
    "StoryPersistenceAdapter",
    "PendingChangesAdapter",
    "ProjectMetaAdapter",
    "TelemetryAdapter",
    "get_modeling_adapters",
    "register_modeling_adapters",

]
