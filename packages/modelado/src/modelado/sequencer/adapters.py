"""
Sequencer Fragment Domain↔Storage Adapters

Bidirectional mappers between sequencer domain models (Pydantic BaseModel)
and storage-layer representations (JSON CAS + metadata).

Responsibilities:
- Domain→Storage: Serialize SequencerFragment/ProjectPhaseFragment to JSON CAS
- Storage→Domain: Deserialize from JSON back to rich Pydantic models

Guarantees:
- Lossless round-trip: storage_to_domain(domain_to_storage(F)) = F
- Deterministic CAS IDs: same content → same Blake3 hash
- Type safety: Pydantic validation on deserialization

Version: 1.0.0 (December 2025)
"""

from __future__ import annotations

import json
import blake3
from typing import Dict, Any, Tuple

from pydantic import ValidationError

from modelado.sequencer.models import (
    SequencerFragment,
    ProjectPhaseFragment,
)


class AdapterError(Exception):
    """Base exception for adapter errors."""
    pass


class SerializationError(AdapterError):
    """Raised when serialization fails."""
    pass


class DeserializationError(AdapterError):
    """Raised when deserialization fails."""
    pass


# ============================================================================
# Storage Fragment (CAS representation)
# ============================================================================

class StorageFragment:
    """Content-addressable storage fragment (simple dataclass for sequencer)."""
    
    def __init__(self, id: str, bytes: bytes, mime_type: str, size: int):
        self.id = id
        self.bytes = bytes
        self.mime_type = mime_type
        self.size = size


# ============================================================================
# Domain → Storage
# ============================================================================

def sequencer_domain_to_storage(
    domain: SequencerFragment
) -> Tuple[StorageFragment, Dict[str, Any]]:
    """Convert domain SequencerFragment to storage CAS blob.
    
    Args:
        domain: Rich Pydantic SequencerFragment
    
    Returns:
        (storage_fragment, metadata) tuple
        - storage_fragment: CAS fragment (id=Blake3 hash, bytes, mime_type, size)
        - metadata: dict with fragment_meta fields (artifact_id, level, type, etc.)
    
    Raises:
        SerializationError: If serialization fails
    """
    try:
        # Serialize to stable JSON (Pydantic handles datetime, UUID, etc.)
        json_dict = domain.model_dump(mode='json', exclude_none=False, by_alias=True)
        
        # Sort keys for determinism
        stable_json = json.dumps(json_dict, sort_keys=True, ensure_ascii=False)
        cas_bytes = stable_json.encode('utf-8')
        
        # Generate Blake3 hash
        fragment_id = blake3.blake3(cas_bytes).hexdigest()
        
        # Create storage fragment
        storage_fragment = StorageFragment(
            id=fragment_id,
            bytes=cas_bytes,
            mime_type="application/vnd.ikam+sequencer+json",
            size=len(cas_bytes)
        )
        
        # Extract metadata
        metadata = {
            "fragment_id": fragment_id,
            "artifact_id": None,  # Will be set by caller if persisting to ikam_artifacts
            "level": 0,  # Sequencer fragments are root-level (no parent)
            "type": "structural",  # Planning artifacts are structural
            "parent_fragment_id": None,
            "salience": domain.confidence_score,  # Use confidence as salience
        }
        
        return storage_fragment, metadata
    
    except Exception as e:
        raise SerializationError(f"Failed to serialize SequencerFragment: {e}") from e


def project_phase_domain_to_storage(
    domain: ProjectPhaseFragment
) -> Tuple[StorageFragment, Dict[str, Any]]:
    """Convert domain ProjectPhaseFragment to storage CAS blob.
    
    Args:
        domain: Rich Pydantic ProjectPhaseFragment
    
    Returns:
        (storage_fragment, metadata) tuple
    
    Raises:
        SerializationError: If serialization fails
    """
    try:
        # Serialize to stable JSON
        json_dict = domain.model_dump(mode='json', exclude_none=False, by_alias=True)
        
        # Sort keys for determinism
        stable_json = json.dumps(json_dict, sort_keys=True, ensure_ascii=False)
        cas_bytes = stable_json.encode('utf-8')
        
        # Generate Blake3 hash
        fragment_id = blake3.blake3(cas_bytes).hexdigest()
        
        # Create storage fragment
        storage_fragment = StorageFragment(
            id=fragment_id,
            bytes=cas_bytes,
            mime_type="application/vnd.ikam+project-phase+json",
            size=len(cas_bytes)
        )
        
        # Extract metadata
        metadata = {
            "fragment_id": fragment_id,
            "artifact_id": None,  # Will be set by caller
            "level": 0,
            "type": "structural",
            "parent_fragment_id": None,
            "salience": 0.8,  # Committed phases have high salience
        }
        
        return storage_fragment, metadata
    
    except Exception as e:
        raise SerializationError(f"Failed to serialize ProjectPhaseFragment: {e}") from e


# ============================================================================
# Storage → Domain
# ============================================================================

def sequencer_storage_to_domain(
    storage: StorageFragment,
    metadata: Dict[str, Any]
) -> SequencerFragment:
    """Reconstruct domain SequencerFragment from storage + metadata.
    
    Args:
        storage: CAS fragment with bytes
        metadata: Metadata dict (not used for reconstruction, but validates round-trip)
    
    Returns:
        Rich Pydantic SequencerFragment
    
    Raises:
        DeserializationError: If deserialization or validation fails
    """
    try:
        # Decode JSON
        json_str = storage.bytes.decode('utf-8')
        json_dict = json.loads(json_str)
        
        # Validate and construct Pydantic model
        domain_fragment = SequencerFragment(**json_dict)
        
        return domain_fragment
    
    except (json.JSONDecodeError, ValidationError, UnicodeDecodeError) as e:
        raise DeserializationError(f"Failed to deserialize SequencerFragment: {e}") from e


def project_phase_storage_to_domain(
    storage: StorageFragment,
    metadata: Dict[str, Any]
) -> ProjectPhaseFragment:
    """Reconstruct domain ProjectPhaseFragment from storage + metadata.
    
    Args:
        storage: CAS fragment with bytes
        metadata: Metadata dict
    
    Returns:
        Rich Pydantic ProjectPhaseFragment
    
    Raises:
        DeserializationError: If deserialization or validation fails
    """
    try:
        # Decode JSON
        json_str = storage.bytes.decode('utf-8')
        json_dict = json.loads(json_str)
        
        # Validate and construct Pydantic model
        domain_fragment = ProjectPhaseFragment(**json_dict)
        
        return domain_fragment
    
    except (json.JSONDecodeError, ValidationError, UnicodeDecodeError) as e:
        raise DeserializationError(f"Failed to deserialize ProjectPhaseFragment: {e}") from e


# ============================================================================
# Public API
# ============================================================================

__all__ = [
    "StorageFragment",
    "sequencer_domain_to_storage",
    "sequencer_storage_to_domain",
    "project_phase_domain_to_storage",
    "project_phase_storage_to_domain",
    "AdapterError",
    "SerializationError",
    "DeserializationError",
]
