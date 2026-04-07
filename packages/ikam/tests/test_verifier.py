"""Tests for DriftSpec, VerificationContract, and VerificationResult schema."""
from dataclasses import FrozenInstanceError
import pytest


def test_driftspec_frozen():
    from ikam.forja.verifier import DriftSpec
    ds = DriftSpec(metric="byte-identity")
    assert ds.metric == "byte-identity"
    assert ds.tolerance == 0.0
    assert ds.params == {}
    with pytest.raises(FrozenInstanceError):
        ds.metric = "changed"


def test_driftspec_pixel_with_params():
    from ikam.forja.verifier import DriftSpec
    ds = DriftSpec(metric="pixel-rmse", tolerance=0.02, params={"dpi": 150})
    assert ds.tolerance == 0.02
    assert ds.params["dpi"] == 150


def test_verification_result_fragment_schema():
    from ikam.forja.verifier import make_verification_result_fragment
    from ikam.forja.verifier import DriftSpec
    from ikam.ir.mime_types import VERIFICATION_RESULT

    ds = DriftSpec(metric="byte-identity")
    frag = make_verification_result_fragment(
        drift_spec=ds,
        measured_drift=0.0,
        passed=True,
        renderer_version="1.0.0",
    )
    assert frag.mime_type == VERIFICATION_RESULT
    assert frag.cas_id is not None
    assert frag.value["passed"] is True
    assert frag.value["measured_drift"] == 0.0
    assert frag.value["drift_spec"]["metric"] == "byte-identity"


def test_verification_result_failed():
    from ikam.forja.verifier import make_verification_result_fragment, DriftSpec

    ds = DriftSpec(metric="pixel-rmse", tolerance=0.01)
    frag = make_verification_result_fragment(
        drift_spec=ds,
        measured_drift=0.05,
        passed=False,
        renderer_version="1.0.0",
        scope="sheet:9",
        retry_count=2,
        diff_summary="Pixel RMSE 0.05 exceeds tolerance 0.01",
    )
    assert frag.value["passed"] is False
    assert frag.value["scope"] == "sheet:9"
    assert frag.value["retry_count"] == 2


def test_verification_contract_protocol():
    from ikam.forja.verifier import VerificationContract, DriftSpec
    from ikam.fragments import Fragment

    class FakeVerifier:
        def verify(self, original, reconstructed, drift_spec):
            return Fragment(value={"passed": True}, mime_type="test/fake")

    assert isinstance(FakeVerifier(), VerificationContract)


def test_byte_identity_verifier_pass():
    from ikam.forja.verifier import ByteIdentityVerifier, DriftSpec
    from ikam.fragments import Fragment

    data = b"hello world"
    original = Fragment(value={"bytes_b64": "aGVsbG8gd29ybGQ="}, mime_type="text/plain")
    verifier = ByteIdentityVerifier()
    ds = DriftSpec(metric="byte-identity")
    result = verifier.verify(original, data, ds)
    assert result.value["passed"] is True


def test_byte_identity_verifier_fail():
    from ikam.forja.verifier import ByteIdentityVerifier, DriftSpec
    from ikam.fragments import Fragment

    original = Fragment(value={"bytes_b64": "aGVsbG8gd29ybGQ="}, mime_type="text/plain")
    verifier = ByteIdentityVerifier()
    ds = DriftSpec(metric="byte-identity")
    result = verifier.verify(original, b"different", ds)
    assert result.value["passed"] is False
    assert result.value["measured_drift"] > 0


def test_verify_reconstruction_no_program():
    from ikam.forja.verifier import verify_reconstruction

    class FakeResult:
        reconstruction_program = None
        source = b"hello"
        structural = []

    report = verify_reconstruction(FakeResult())
    assert report.passed is False
    assert "No reconstruction program" in report.reason
