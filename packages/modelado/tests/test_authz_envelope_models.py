import pytest
from pydantic import ValidationError

from modelado.authz.envelope import (
    SignatureVerificationDecision,
    SignedMutationIntent,
    VerificationReason,
)


def _valid_intent(**overrides):
    base = dict(
        operation="ikam.graph.write",
        project_id="project-123",
        timestamp_ms=1700000000000,
        nonce="nonce-abc",
        payload_hash="sha256:" + ("a" * 64),
        signature="ed25519:ZmFrZS1zaWduYXR1cmU",  # base64url("fake-signature")
    )
    base.update(overrides)
    return SignedMutationIntent(**base)


def test_signed_mutation_intent_signing_bytes_deterministic():
    intent = _valid_intent()
    assert intent.signing_bytes() == intent.signing_bytes()


def test_signed_mutation_intent_signature_not_included_in_signing_payload():
    intent_a = _valid_intent(signature="ed25519:ZmFrZS1zaWduYXR1cmUtYQ")
    intent_b = _valid_intent(signature="ed25519:ZmFrZS1zaWduYXR1cmUtYg")
    assert intent_a.signing_bytes() == intent_b.signing_bytes()


def test_signed_mutation_intent_payload_hash_validation():
    with pytest.raises(ValidationError):
        _valid_intent(payload_hash="sha256:bad")

    with pytest.raises(ValidationError):
        _valid_intent(payload_hash="md5:" + ("a" * 32))


def test_signed_mutation_intent_signature_validation():
    with pytest.raises(ValidationError):
        _valid_intent(signature="rsa:abc")

    with pytest.raises(ValidationError):
        _valid_intent(signature="")


def test_signed_mutation_intent_required_fields():
    with pytest.raises(ValidationError):
        SignedMutationIntent(
            operation="x",
            project_id="p",
            timestamp_ms=1,
            nonce="n",
            payload_hash="sha256:" + ("a" * 64),
            # signature missing
        )


def test_signature_verification_decision_canonical_bytes_deterministic_and_sorts_details_keys():
    decision = SignatureVerificationDecision(
        allowed=False,
        reason=VerificationReason.INVALID_SIGNATURE,
        key_fingerprint=None,
        details={"z": 1, "a": 2},
    )

    b1 = decision.canonical_bytes()
    b2 = decision.canonical_bytes()
    assert b1 == b2

    # Ensure canonical sorting of nested dict keys is effective.
    # If keys are sorted, "a" should appear before "z" in the JSON text.
    text = b1.decode("utf-8")
    assert text.index('"a"') < text.index('"z"')
