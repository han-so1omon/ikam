from __future__ import annotations

from enum import Enum
from typing import Any, Dict, Optional

from pydantic import BaseModel, ConfigDict, Field, field_validator

from .signing import SigningError, canonical_json_bytes, parse_ed25519_signature


class VerificationReason(str, Enum):
    VALID = "valid"
    INVALID_SIGNATURE = "invalid_signature"
    INVALID_ENVELOPE = "invalid_envelope"
    KEY_NOT_FOUND = "key_not_found"
    KEY_REVOKED = "key_revoked"
    REPLAY = "replay"
    ERROR = "error"


class SignedMutationIntent(BaseModel):
    """Canonical signed envelope describing an automation mutation intent.

    This model is DB-independent: it does not perform agent key lookups nor
    anti-replay persistence. It defines the canonical payload used for signing.

    Signature is expected to be of the form `ed25519:<b64url>`.
    """

    operation: str = Field(..., min_length=1, description="Mutation operation identifier")
    project_id: str = Field(..., min_length=1, description="Project ID in scope")
    timestamp_ms: int = Field(..., ge=0, description="Unix epoch timestamp (ms)")
    nonce: str = Field(..., min_length=1, description="Anti-replay nonce/idempotency key")
    payload_hash: str = Field(..., min_length=1, description="Payload hash string (e.g. sha256:<hex>)")
    signature: str = Field(..., min_length=1, description="Signature string (ed25519:<b64url>)")

    model_config = ConfigDict(extra="forbid", frozen=True)

    @field_validator("payload_hash")
    @classmethod
    def _validate_payload_hash(cls, value: str) -> str:
        if not isinstance(value, str) or not value.startswith("sha256:"):
            raise ValueError("payload_hash must be in the form sha256:<hex>")
        hex_part = value.split(":", 1)[1]
        if len(hex_part) != 64:
            raise ValueError("payload_hash sha256 hex must be 64 characters")
        try:
            int(hex_part, 16)
        except ValueError as exc:
            raise ValueError("payload_hash sha256 hex must be lowercase hex") from exc
        return value

    @field_validator("signature")
    @classmethod
    def _validate_signature(cls, value: str) -> str:
        if not isinstance(value, str) or not value.startswith("ed25519:"):
            raise ValueError("signature must be in the form ed25519:<b64url>")
        return value

    def signing_payload_dict(self) -> Dict[str, Any]:
        """Return the canonical dict that is signed.

        The signature itself is excluded by design.
        """

        return {
            "nonce": self.nonce,
            "operation": self.operation,
            "payload_hash": self.payload_hash,
            "project_id": self.project_id,
            "timestamp_ms": self.timestamp_ms,
        }

    def signing_bytes(self) -> bytes:
        """Return canonical JSON bytes of the signing payload."""

        return canonical_json_bytes(self.signing_payload_dict())

    def signature_bytes(self) -> bytes:
        """Parse the signature string into raw signature bytes."""

        return parse_ed25519_signature(self.signature)


class SignatureVerificationDecision(BaseModel):
    """Structured record of signature verification outcome.

    This record is suitable for later enrichment (key fingerprint, agent_id,
    policy refs) and for use when emitting SystemMutated audits.
    """

    allowed: bool = Field(..., description="Whether the envelope is permitted")
    reason: VerificationReason = Field(..., description="Decision reason code")
    key_fingerprint: Optional[str] = Field(
        default=None,
        description="Fingerprint of the key used/attempted (if known)",
    )
    agent_id: Optional[str] = Field(default=None, description="Agent identifier (if known)")
    verified_at_ms: Optional[int] = Field(default=None, ge=0, description="Unix epoch timestamp (ms)")
    details: Optional[Dict[str, Any]] = Field(default=None, description="Structured debugging/audit details")

    model_config = ConfigDict(extra="forbid", frozen=True)

    def canonical_bytes(self) -> bytes:
        """Return canonical JSON bytes for deterministic audit storage/hashing."""

        try:
            payload = self.model_dump(mode="json", exclude_none=True)
            return canonical_json_bytes(payload)
        except SigningError:
            raise
        except Exception as exc:  # noqa: BLE001
            raise SigningError(f"Decision is not serializable for signing: {exc}") from exc


__all__ = [
    "VerificationReason",
    "SignedMutationIntent",
    "SignatureVerificationDecision",
]
