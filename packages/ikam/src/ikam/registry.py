from __future__ import annotations
from typing import Dict, Generic, TypeVar, Optional, List, Any

T = TypeVar("T")

class Registry(Generic[T]):
    """Generic registry for pluggable components, strategies, or profiles.
    
    Provides a simple key-value store with support for registration and retrieval.
    """
    def __init__(self, name: str):
        self.name = name
        self._entries: Dict[str, T] = {}

    def register(self, key: str, entry: T) -> None:
        """Register a new entry under the given key."""
        self._entries[key] = entry

    def get(self, key: str) -> Optional[T]:
        """Retrieve the entry for the given key, or None if not found."""
        return self._entries.get(key)

    def list_keys(self) -> List[str]:
        """Return a list of all registered keys."""
        return list(self._entries.keys())

    def clear(self) -> None:
        """Clear all entries in the registry."""
        self._entries.clear()

    def __contains__(self, key: str) -> bool:
        return key in self._entries

    def __getitem__(self, key: str) -> T:
        return self._entries[key]
