"""Interacciones Streaming - SSE and Kafka streaming utilities for real-time event broadcasting.

Provides:
- `SSEConnectionManager` — Manage SSE connections and broadcast events
- `KafkaEventStreamer` — Stream events from Kafka topics
- `EventReplayBuffer` — In-memory replay buffer for late subscribers

Example:
    >>> from interacciones.streaming import SSEConnectionManager
    >>> manager = SSEConnectionManager()
    >>> await manager.broadcast({"event": "node_start", "node_id": "parser"})
"""

# Core streaming components (implemented later)
# from .sse import SSEConnectionManager
# from .kafka import KafkaEventStreamer
# from .replay import EventReplayBuffer

# __all__ = [
#     "SSEConnectionManager",
#     "KafkaEventStreamer",
#     "EventReplayBuffer",
# ]

# Placeholder during development
__all__ = []
