import pytest


def test_validate_provenance_event_details_rendered_requires_metadata() -> None:
    from modelado.ikam_graph_repository import validate_provenance_event_details

    with pytest.raises(ValueError, match="Rendered event requires details keys"):
        validate_provenance_event_details("Rendered", details={"seed": 1})


def test_validate_provenance_event_details_system_mutated_requires_signed_intent_fields() -> None:
    from modelado.ikam_graph_repository import validate_provenance_event_details

    base = {
        "operation": "graph_edge.upsert",
        "project_id": "proj_test",
        "timestamp": "2026-01-23T00:00:00Z",
        "nonce": "nonce_123",
        "payload_hash": "blake3:deadbeef",
        "agent_id": "agent_test",
        "key_fingerprint": "blake3:feedface",
        "signature": "ed25519:abcd",
    }

    validate_provenance_event_details("SystemMutated", details=base)

    missing = dict(base)
    missing.pop("signature")
    with pytest.raises(ValueError, match="SystemMutated event requires details keys"):
        validate_provenance_event_details("SystemMutated", details=missing)
