from __future__ import annotations

import base64
import hashlib
import json
from typing import Any

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey


class SigningError(ValueError):
    pass


def canonical_json_bytes(obj: Any) -> bytes:
    """Serialize `obj` to canonical JSON bytes.

    Canonicalization rules:
    - UTF-8 encoded
    - Keys sorted
    - No insignificant whitespace
    - Rejects NaN/Infinity

    This is intended for deterministic hashing/signing.
    """

    try:
        text = json.dumps(
            obj,
            sort_keys=True,
            separators=(",", ":"),
            ensure_ascii=False,
            allow_nan=False,
        )
    except (TypeError, ValueError) as exc:
        raise SigningError(f"Object is not JSON-serializable for signing: {exc}") from exc

    return text.encode("utf-8")


def sha256_payload_hash(payload_bytes: bytes) -> str:
    """Compute a stable SHA256 payload hash string."""

    digest = hashlib.sha256(payload_bytes).hexdigest()
    return f"sha256:{digest}"


def b64url_encode(data: bytes) -> str:
    """Base64url without padding."""

    return base64.urlsafe_b64encode(data).rstrip(b"=").decode("ascii")


def b64url_decode(data: str) -> bytes:
    """Decode base64url that may omit padding."""

    if not isinstance(data, str) or not data:
        raise SigningError("b64url value must be a non-empty string")

    padded = data + ("=" * (-len(data) % 4))
    try:
        return base64.urlsafe_b64decode(padded.encode("ascii"))
    except Exception as exc:  # noqa: BLE001
        raise SigningError("Invalid base64url value") from exc


def parse_ed25519_signature(signature: str) -> bytes:
    """Parse signatures of the form `ed25519:<b64url>` into raw bytes."""

    if not isinstance(signature, str) or not signature.startswith("ed25519:"):
        raise SigningError("Signature must be in the form ed25519:<b64url>")

    return b64url_decode(signature.split(":", 1)[1])


def is_valid_ed25519_signature(
    *, public_key_bytes: bytes, message: bytes, signature_bytes: bytes
) -> bool:
    """Return True iff the signature is valid for the given message."""

    try:
        Ed25519PublicKey.from_public_bytes(public_key_bytes).verify(signature_bytes, message)
        return True
    except InvalidSignature:
        return False
    except Exception as exc:  # noqa: BLE001
        raise SigningError("Invalid public key bytes or signature bytes") from exc


def is_valid_ed25519_signature_string(
    *, public_key_bytes: bytes, message: bytes, signature: str
) -> bool:
    """Validate `ed25519:<b64url>` signature strings."""

    return is_valid_ed25519_signature(
        public_key_bytes=public_key_bytes,
        message=message,
        signature_bytes=parse_ed25519_signature(signature),
    )
