# packages/ikam/tests/test_graph_model_cleanup.py
"""Verify legacy graph model types are removed and kept types survive."""
import pytest


def test_artifact_still_exists():
    from ikam.graph import Artifact, ArtifactKind
    assert Artifact is not None
    assert ArtifactKind is not None


def test_fragment_stored_fragment_exists():
    from ikam.graph import StoredFragment
    assert StoredFragment is not None


def test_fragment_name_removed_from_graph_module():
    with pytest.raises(ImportError):
        from ikam.graph import Fragment  # noqa: F401


def test_provenance_event_still_exists():
    from ikam.graph import ProvenanceEvent
    assert ProvenanceEvent is not None


def test_cas_hex_still_exists():
    from ikam.graph import _cas_hex
    assert callable(_cas_hex)


def test_derivation_removed():
    with pytest.raises(ImportError):
        from ikam.graph import Derivation  # noqa: F401


def test_derivation_type_removed():
    with pytest.raises(ImportError):
        from ikam.graph import DerivationType  # noqa: F401


def test_action_type_removed():
    with pytest.raises(ImportError):
        from ikam.graph import ActionType  # noqa: F401


def test_variation_removed():
    with pytest.raises(ImportError):
        from ikam.graph import Variation  # noqa: F401


def test_get_reconstruction_entrypoint_removed():
    with pytest.raises(ImportError):
        from ikam.graph import get_reconstruction_entrypoint  # noqa: F401


def test_artifact_has_no_fragment_ids():
    from ikam.graph import Artifact
    a = Artifact(id="test", kind="document")
    assert not hasattr(a, "fragment_ids")
