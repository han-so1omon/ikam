from modelado.ikam_artifact_store_pg import PostgresArtifactStore


class _RecordingConnection:
    def __init__(self) -> None:
        self.calls: list[tuple[str, tuple[object, ...]]] = []

    def execute(self, query: str, params: tuple[object, ...]):
        self.calls.append((query, params))
        return None


def test_insert_fragment_object_uses_current_manifest_adapter_signature(monkeypatch):
    captured: dict[str, object] = {}

    def fake_build_fragment_object_manifest(*, artifact_id: str, kind: str, fragment_ids: list[str]):
        captured["artifact_id"] = artifact_id
        captured["kind"] = kind
        captured["fragment_ids"] = fragment_ids
        return {
            "schemaVersion": 1,
            "artifactId": artifact_id,
            "kind": kind,
            "fragments": [{"fragmentId": fragment_id} for fragment_id in fragment_ids],
        }

    monkeypatch.setattr(
        "modelado.ikam_artifact_store_pg.build_fragment_object_manifest",
        fake_build_fragment_object_manifest,
    )

    store = PostgresArtifactStore(_RecordingConnection())

    store._insert_fragment_object(
        artifact_id="00000000-0000-0000-0000-000000000000",
        kind="document",
        fragment_ids=["fragment-1", "fragment-2"],
        fragments=[object()],
        domain_id_to_cas_id={"fragment-1": "cas-1"},
    )

    assert captured == {
        "artifact_id": "00000000-0000-0000-0000-000000000000",
        "kind": "document",
        "fragment_ids": ["fragment-1", "fragment-2"],
    }


def test_upsert_artifact_head_ref_records_branch_head_state() -> None:
    cx = _RecordingConnection()
    store = PostgresArtifactStore(cx)

    store.upsert_artifact_head_ref(
        artifact_id="00000000-0000-0000-0000-000000000000",
        ref="refs/heads/main",
        head_object_id="obj-head-1",
        head_commit_id="iac-main-1",
    )

    assert len(cx.calls) == 1
    query, params = cx.calls[0]
    assert "INSERT INTO ikam_artifact_branches" in query
    assert "INSERT INTO ikam_artifact_commits" in query
    assert "head_commit_id" in query
    assert params == (
        "00000000-0000-0000-0000-000000000000",
        "iac-main-1",
        '{"ref": null}',
        '{"commit_id":"iac-main-1","head_object_id":"obj-head-1","ref":"refs/heads/main"}',
        "iac-main-1",
        "obj-head-1",
        "00000000-0000-0000-0000-000000000000",
        "00000000-0000-0000-0000-000000000000",
        "main",
        "iac-main-1",
    )
