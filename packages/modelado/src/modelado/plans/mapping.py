from __future__ import annotations
import hashlib
import json
from typing import Any, Dict, List, Literal, Optional, Union

from pydantic import BaseModel, Field, ConfigDict
try:
    from blake3 import blake3 as _blake3

    def _blake3_hexdigest(data: bytes) -> str:
        return _blake3(data).hexdigest()
except ImportError:
    import hashlib

    def _blake3_hexdigest(data: bytes) -> str:
        return hashlib.blake2b(data).hexdigest()

STRUCTURAL_MAP_SCHEMA_ID = "modelado/structural-map@1"

class MapDNA(BaseModel):
    """A projective fingerprint (structural hashes) used for neighborhood retrieval."""
    model_config = ConfigDict(extra="forbid")

    fingerprint: str = Field(..., description="Global structural fingerprint (hash of the entire structure)")
    structural_hashes: List[str] = Field(
        default_factory=list, 
        description="List of component hashes (one per node or structural unit) for granular matching"
    )
    
    version: str = "1"

class StructuralMapNode(BaseModel):
    """A node in the structural hierarchy (e.g., a section, slide, or page)."""
    model_config = ConfigDict(extra="forbid")

    id: str = Field(..., description="Unique ID for this node within the map")
    title: str = Field(..., description="Title or label for this structural element")
    level: int = Field(..., description="Hierarchy level (0 for root, 1 for top-level sections, etc.)")
    kind: str = Field(..., description="Kind of structural element (e.g., 'section', 'slide', 'table', 'paragraph')")
    
    description: Optional[str] = Field(None, description="Optional semantic summary of this node's content")
    source_range: Optional[Dict[str, Any]] = Field(None, description="Reference to the original source location (offsets, pages, etc.)")
    
    children: List[StructuralMapNode] = Field(default_factory=list, description="Ordered child nodes")
    metadata: Dict[str, Any] = Field(default_factory=dict, description="Arbitrary extension metadata")

class StructuralMap(BaseModel):
    """A hierarchy (TOC/Outline) artifact that guides downstream agents."""
    model_config = ConfigDict(extra="forbid")

    schema_id: Literal["modelado/structural-map@1"] = Field(
        default=STRUCTURAL_MAP_SCHEMA_ID,
        alias="schema",
        description="Structural map schema id",
    )

    artifact_id: str = Field(..., description="ID of the artifact this map describes")
    root: StructuralMapNode = Field(..., description="Root node of the structural hierarchy")
    
    # Map DNA is often stored alongside the map for fast retrieval
    dna: Optional[MapDNA] = None
    
    metadata: Dict[str, Any] = Field(default_factory=dict)

def compute_map_dna(structural_map: StructuralMap) -> MapDNA:
    """
    Computes the Structural DNA for a given StructuralMap.
    The DNA consists of a global fingerprint and a list of granular hashes.
    Uses blake3 for consistent hashing with the IKAM CAS layer.
    """
    hashes: List[str] = []
    
    def traverse(node: StructuralMapNode):
        # Fingerprint components for this node: level, title, kind, and number of children
        # We use a stable JSON-like representation for hashing
        components = {
            "l": node.level,
            "t": node.title,
            "k": node.kind,
            "c": len(node.children)
        }
        blob = json.dumps(components, sort_keys=True, separators=(",", ":")).encode("utf-8")
        node_hash = _blake3_hexdigest(blob)[:16]  # Truncated for granular DNA
        hashes.append(node_hash)
        
        for child in node.children:
            traverse(child)
            
    traverse(structural_map.root)
    
    # Global fingerprint is a hash of all node hashes in order
    global_blob = "".join(hashes).encode("utf-8")
    global_fingerprint = _blake3_hexdigest(global_blob)
    
    return MapDNA(fingerprint=global_fingerprint, structural_hashes=hashes, version="1")

def canonicalize_structural_map_json(structural_map: Union[StructuralMap, Dict[str, Any]]) -> bytes:
    """Serialize a structural map to deterministic UTF-8 JSON bytes."""
    model = structural_map if isinstance(structural_map, StructuralMap) else StructuralMap.model_validate(structural_map)
    payload = model.model_dump(mode="json", by_alias=True, exclude_none=True)
    stable_json = json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return stable_json.encode("utf-8")
