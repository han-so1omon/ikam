# Storage layer models (CAS persistence only)
from ikam.graph import StoredFragment, _cas_hex, Artifact


def test_fragment_cas_and_roundtrip():
    payload = b"hello-ikam"
    f = StoredFragment.from_bytes(payload, mime_type="text/plain")
    assert f.id == _cas_hex(payload)
    assert f.bytes == payload
    assert f.size == len(payload)
    assert f.mime_type == "text/plain"


def test_artifact_root_fragment_linkage():
    """Artifact uses root_fragment_id (not fragment_ids) for DAG entrypoint."""
    payload = b"A"
    f = StoredFragment.from_bytes(payload)
    art = Artifact(id="art-1", kind="file", root_fragment_id=f.id)
    assert art.root_fragment_id == f.id
    assert art.kind == "file"
