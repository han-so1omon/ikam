"""Verification contracts for the compression/re-render pipeline.

DriftSpec: parameter specifying how to measure drift.
VerificationContract: protocol for verifiers.
VerificationResult: Fragment with MIME application/ikam-verification-result+json.
ByteIdentityVerifier: built-in exact-bytes verifier.
"""
from __future__ import annotations

import base64
import datetime as dt
from dataclasses import dataclass, field
from typing import Any, Dict, Optional, Protocol, runtime_checkable

from ikam.fragments import Fragment
from ikam.forja.cas import cas_fragment
from ikam.ir.mime_types import VERIFICATION_RESULT

from blake3 import blake3 as _hash


@dataclass(frozen=True)
class DriftSpec:
    """Specifies how to measure drift between expected and actual output."""
    metric: str             # "byte-identity", "pixel-rmse", "content-stream-eq", "ir-canonical-eq"
    tolerance: float = 0.0  # 0.0 for exact match; >0 for fuzzy
    params: Dict[str, Any] = field(default_factory=dict)


@runtime_checkable
class VerificationContract(Protocol):
    """Verifies that reconstruction produces acceptable output."""
    def verify(
        self,
        original: Fragment,
        reconstructed: bytes,
        drift_spec: DriftSpec,
    ) -> Fragment: ...


def make_verification_result_fragment(
    *,
    drift_spec: DriftSpec,
    measured_drift: float,
    passed: bool,
    renderer_version: str,
    scope: Optional[str] = None,
    retry_count: int = 0,
    diff_summary: Optional[str] = None,
) -> Fragment:
    """Create a VerificationResult Fragment."""
    value = {
        "drift_spec": {
            "metric": drift_spec.metric,
            "tolerance": drift_spec.tolerance,
            "params": drift_spec.params,
        },
        "measured_drift": measured_drift,
        "passed": passed,
        "renderer_version": renderer_version,
        "scope": scope,
        "retry_count": retry_count,
        "diff_summary": diff_summary,
        "verified_at": dt.datetime.now(dt.timezone.utc).isoformat(),
    }
    return cas_fragment(value, VERIFICATION_RESULT)


class ByteIdentityVerifier:
    """Exact byte-level verification via BLAKE3 hash comparison."""

    def verify(
        self,
        original: Fragment,
        reconstructed: bytes,
        drift_spec: DriftSpec,
    ) -> Fragment:
        # Extract original bytes from Fragment value
        original_b64 = original.value.get("bytes_b64", "") if original.value else ""
        original_bytes = base64.b64decode(original_b64)

        original_hash = _hash(original_bytes).hexdigest()
        reconstructed_hash = _hash(reconstructed).hexdigest()

        passed = original_hash == reconstructed_hash
        measured_drift = 0.0 if passed else 1.0

        return make_verification_result_fragment(
            drift_spec=drift_spec,
            measured_drift=measured_drift,
            passed=passed,
            renderer_version="1.0.0",
            diff_summary=None if passed else f"Hash mismatch: {original_hash[:16]}... != {reconstructed_hash[:16]}...",
        )


@dataclass
class VerificationReport:
    """Result of verify_reconstruction() — simple pass/fail with reason."""
    passed: bool
    reason: Optional[str] = None


def verify_reconstruction(decomposition_result) -> VerificationReport:
    """Convenience function: verify a DecompositionResult has a valid reconstruction program.

    Checks that the result's reconstruction program, when applied, reproduces
    the original source bytes. Uses ByteIdentityVerifier with byte-identity metric.

    Args:
        decomposition_result: A DecompositionResult with .source (original bytes),
            .structural (list of Fragments), and .reconstruction_program (optional).

    Returns:
        VerificationReport with passed=True if reconstruction is byte-identical.
    """
    if not hasattr(decomposition_result, "reconstruction_program") or decomposition_result.reconstruction_program is None:
        return VerificationReport(passed=False, reason="No reconstruction program in result")

    if not hasattr(decomposition_result, "source") or decomposition_result.source is None:
        return VerificationReport(passed=False, reason="No source bytes to verify against")

    try:
        program = decomposition_result.reconstruction_program
        reconstructed = program.execute(decomposition_result.structural)
        original = decomposition_result.source

        original_hash = _hash(original).hexdigest()
        reconstructed_hash = _hash(reconstructed).hexdigest()

        if original_hash == reconstructed_hash:
            return VerificationReport(passed=True)
        else:
            return VerificationReport(
                passed=False,
                reason=f"Hash mismatch: {original_hash[:16]}... != {reconstructed_hash[:16]}...",
            )
    except Exception as e:
        return VerificationReport(passed=False, reason=f"Reconstruction failed: {e}")
