from ikam.ingest_uploaded import ingest_uploaded_file


def test_ingest_uploaded_file_produces_artifact_and_fragment():
    payload = b"binary-data-001"
    art, frag = ingest_uploaded_file(payload, mime_type="application/octet-stream", artifact_id="art-up-1", title="Upload")
    assert frag.bytes == payload
    assert art.root_fragment_id == frag.id
    assert art.kind == "file"
