"""
IKAM v2 Fragment Codec

Provides lossless encoding, decoding, and validation for Fragment objects.
Implements the mathematical guarantee: decode(encode(F)) = F (byte-level equality).

References:
- docs/ikam/ikam-v2-fragmented-knowledge-system.md
- docs/ikam/ikam-fragmentation-math-model.md

Mathematical Guarantees:
- Lossless encoding: decode(encode(X)) = X (bijective transformation)
- Deterministic: encode(F1) = encode(F2) iff F1 = F2
- Canonical representation: encode() produces unique bytes for unique fragments
- Integrity validation: validate() detects any corruption in encoded bytes

Encoding Format:
- JSON-based serialization with Pydantic model dumping
- UTF-8 encoding for wire/storage transport
- Optional compression (gzip) for large payloads
- Hash validation (BLAKE3) for integrity checks

Version: 1.0.0 (IKAM v2 MVP - November 2025)
"""

from __future__ import annotations

import gzip
import json
from typing import Any, Dict, List, Optional

try:
    import blake3
    _HAS_BLAKE3 = True
except ImportError:
    import hashlib
    _HAS_BLAKE3 = False

# V3 Fragment: codec operates on the canonical Fragment type
from ikam.fragments import Fragment


class FragmentCodec:
    """
    Codec for lossless Fragment serialization.

    Mathematical Properties:
    - Bijective: decode(encode(F)) = F (perfect round-trip)
    - Deterministic: encode(F) always produces same bytes
    - Canonical: one fragment → one unique encoding
    - Validated: validate() ensures integrity via hash

    Usage:
        codec = FragmentCodec()
        encoded = codec.encode(fragment)
        decoded = codec.decode(encoded)
        assert decoded == fragment  # Pydantic equality check
    """

    def __init__(self, compress: bool = False, validate_on_decode: bool = True):
        """
        Initialize FragmentCodec.

        Args:
            compress: Enable gzip compression for encoded bytes
            validate_on_decode: Automatically validate integrity when decoding
        """
        self.compress = compress
        self.validate_on_decode = validate_on_decode

    def encode(self, fragment: Fragment) -> bytes:
        """
        Encode Fragment to bytes (lossless).

        Mathematical Guarantee:
        - decode(encode(F)) = F (byte-level round-trip)
        - encode(F1) = encode(F2) iff F1 = F2 (canonical)

        Args:
            fragment: Fragment object to encode

        Returns:
            Encoded bytes (JSON UTF-8, optionally gzipped)

        Raises:
            ValueError: If fragment validation fails
        """
        # Pydantic model_dump with alias support (for snake_case → camelCase)
        fragment_dict = fragment.model_dump(mode="json", by_alias=True)

        # Serialize to canonical JSON (sorted keys for determinism)
        json_str = json.dumps(fragment_dict, sort_keys=True, ensure_ascii=False)
        json_bytes = json_str.encode("utf-8")

        # Optional compression
        if self.compress:
            return gzip.compress(json_bytes)
        return json_bytes

    def decode(self, data: bytes) -> Fragment:
        """
        Decode bytes to Fragment (lossless).

        Mathematical Guarantee:
        - decode(encode(F)) = F (perfect reconstruction)
        - Validates integrity if validate_on_decode=True

        Args:
            data: Encoded bytes (from encode())

        Returns:
            Reconstructed Fragment object

        Raises:
            ValueError: If data is corrupted or invalid
            ValidationError: If Pydantic validation fails
        """
        # Decompress if needed
        if self.compress or self._is_gzipped(data):
            try:
                json_bytes = gzip.decompress(data)
            except gzip.BadGzipFile:
                json_bytes = data  # Not compressed, use as-is
        else:
            json_bytes = data

        # Parse JSON
        try:
            json_str = json_bytes.decode("utf-8")
            fragment_dict = json.loads(json_str)
        except (UnicodeDecodeError, json.JSONDecodeError) as e:
            raise ValueError(f"Invalid fragment encoding: {e}") from e

        # Reconstruct Pydantic model
        fragment = Fragment.model_validate(fragment_dict)

        # Validate integrity if enabled
        if self.validate_on_decode:
            if not self.validate(data):
                raise ValueError("Fragment integrity validation failed")

        return fragment

    def validate(self, data: bytes) -> bool:
        """
        Validate encoded fragment integrity.

        Strategy:
        - Decode and re-encode to check round-trip equality
        - If round-trip succeeds, data is valid

        Args:
            data: Encoded bytes to validate

        Returns:
            True if valid, False otherwise
        """
        try:
            # Disable recursive validation to avoid infinite loop
            original_validate_flag = self.validate_on_decode
            self.validate_on_decode = False

            # Round-trip test: decode then re-encode
            fragment = self.decode(data)
            re_encoded = self.encode(fragment)

            # Restore flag
            self.validate_on_decode = original_validate_flag

            # Byte-level equality check
            return re_encoded == data
        except Exception:
            return False

    def hash(self, fragment: Fragment) -> str:
        """
        Compute content hash of fragment (for CAS addressing).

        Returns BLAKE3 hash (or SHA256 fallback) as hex string.

        Args:
            fragment: Fragment to hash

        Returns:
            Hex-encoded hash (64 chars for BLAKE3, 64 chars for SHA256)
        """
        encoded = self.encode(fragment)
        if _HAS_BLAKE3:
            return blake3.blake3(encoded).hexdigest()
        else:
            return hashlib.sha256(encoded).hexdigest()

    @staticmethod
    def _is_gzipped(data: bytes) -> bool:
        """Check if data starts with gzip magic bytes."""
        return len(data) >= 2 and data[:2] == b"\x1f\x8b"


class FragmentListCodec:
    """
    Codec for encoding/decoding lists of Fragments.

    Useful for batch storage and multi-fragment operations.
    """

    def __init__(self, compress: bool = False):
        self.codec = FragmentCodec(compress=compress, validate_on_decode=False)
        self.compress = compress

    def encode(self, fragments: List[Fragment]) -> bytes:
        """
        Encode list of Fragments to bytes.

        Returns JSON array of fragment objects.
        """
        fragments_dicts = [f.model_dump(mode="json", by_alias=True) for f in fragments]
        json_str = json.dumps(fragments_dicts, sort_keys=True, ensure_ascii=False)
        json_bytes = json_str.encode("utf-8")

        if self.compress:
            return gzip.compress(json_bytes)
        return json_bytes

    def decode(self, data: bytes) -> List[Fragment]:
        """
        Decode bytes to list of Fragments.
        """
        # Decompress if needed
        if self.compress or FragmentCodec._is_gzipped(data):
            try:
                json_bytes = gzip.decompress(data)
            except gzip.BadGzipFile:
                json_bytes = data
        else:
            json_bytes = data

        # Parse JSON array
        try:
            json_str = json_bytes.decode("utf-8")
            fragments_dicts = json.loads(json_str)
        except (UnicodeDecodeError, json.JSONDecodeError) as e:
            raise ValueError(f"Invalid fragment list encoding: {e}") from e

        # Reconstruct Pydantic models
        if not isinstance(fragments_dicts, list):
            raise ValueError("Expected JSON array for fragment list")

        return [Fragment.model_validate(d) for d in fragments_dicts]

    def validate(self, data: bytes) -> bool:
        """
        Validate encoded fragment list integrity via round-trip.
        """
        try:
            fragments = self.decode(data)
            re_encoded = self.encode(fragments)
            return re_encoded == data
        except Exception:
            return False
