import hashlib

import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from modelado.authz.signing import (
    SigningError,
    b64url_decode,
    b64url_encode,
    canonical_json_bytes,
    is_valid_ed25519_signature,
    is_valid_ed25519_signature_string,
    parse_ed25519_signature,
    sha256_payload_hash,
)


def test_canonical_json_bytes_is_deterministic_for_key_order():
    a = {"b": 2, "a": 1}
    b = {"a": 1, "b": 2}
    assert canonical_json_bytes(a) == canonical_json_bytes(b)


def test_canonical_json_bytes_rejects_nan():
    with pytest.raises(SigningError):
        canonical_json_bytes({"x": float("nan")})


def test_sha256_payload_hash_matches_hashlib():
    payload = b"hello"
    expected = "sha256:" + hashlib.sha256(payload).hexdigest()
    assert sha256_payload_hash(payload) == expected


def test_b64url_roundtrip():
    raw = b"\x00\x01\x02hello\xff"
    assert b64url_decode(b64url_encode(raw)) == raw


def test_ed25519_signature_validates_bytes_and_string_forms():
    priv = Ed25519PrivateKey.generate()
    pub_bytes = priv.public_key().public_bytes_raw()

    msg = canonical_json_bytes({"op": "mutate", "n": 1})
    sig_bytes = priv.sign(msg)

    assert is_valid_ed25519_signature(
        public_key_bytes=pub_bytes, message=msg, signature_bytes=sig_bytes
    )

    sig_str = "ed25519:" + b64url_encode(sig_bytes)
    assert is_valid_ed25519_signature_string(
        public_key_bytes=pub_bytes, message=msg, signature=sig_str
    )

    assert parse_ed25519_signature(sig_str) == sig_bytes


def test_ed25519_signature_rejects_invalid_signature():
    priv_good = Ed25519PrivateKey.generate()
    priv_bad = Ed25519PrivateKey.generate()

    msg = canonical_json_bytes({"x": 1})
    sig_bad = priv_bad.sign(msg)

    assert not is_valid_ed25519_signature(
        public_key_bytes=priv_good.public_key().public_bytes_raw(),
        message=msg,
        signature_bytes=sig_bad,
    )


def test_parse_ed25519_signature_rejects_wrong_prefix():
    with pytest.raises(SigningError):
        parse_ed25519_signature("hmac:abcd")
