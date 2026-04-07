"""Authorization helpers.

This package contains pure utilities used by authorization/auditing flows.
"""

from .envelope import (  # noqa: F401
	SignatureVerificationDecision,
	SignedMutationIntent,
	VerificationReason,
)

from .signing import (  # noqa: F401
	SigningError,
	b64url_decode,
	b64url_encode,
	canonical_json_bytes,
	is_valid_ed25519_signature,
	is_valid_ed25519_signature_string,
	parse_ed25519_signature,
	sha256_payload_hash,
)

__all__ = [
	"SigningError",
	"b64url_decode",
	"b64url_encode",
	"canonical_json_bytes",
	"is_valid_ed25519_signature",
	"is_valid_ed25519_signature_string",
	"parse_ed25519_signature",
	"sha256_payload_hash",
	"VerificationReason",
	"SignedMutationIntent",
	"SignatureVerificationDecision",
]
