"""Integration tests for rewritten ikam_graph_repository."""
import pytest
from unittest.mock import MagicMock, patch, call


@patch("modelado.ikam_graph_repository.append_graph_edge_event")
@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_full_ingestion_flow(mock_require, mock_append):
    """Simulate: decompose → store fragments → emit edges → record provenance → promote."""
    from modelado.ikam_graph_repository import (
        store_fragment,
        emit_edge_event,
        record_provenance_event,
    )
    from ikam.forja.cas import cas_fragment

    cx = MagicMock()
    cursor = MagicMock()
    cx.cursor.return_value.__enter__ = MagicMock(return_value=cursor)
    cx.cursor.return_value.__exit__ = MagicMock(return_value=False)

    # 1. Create fragments
    root = cas_fragment({"text": "# Doc"}, "text/markdown")
    child = cas_fragment({"text": "paragraph"}, "text/ikam-paragraph")

    # 2. Store against a branch ref
    store_fragment(cx, root, project_id="proj1", ref="refs/heads/run/run-123")
    store_fragment(cx, child, project_id="proj1", ref="refs/heads/run/run-123")

    # 3. Emit edge
    emit_edge_event(
        cx,
        source_id=root.cas_id,
        target_id=child.cas_id,
        predicate="contains",
        project_id="proj1",
        ref="refs/heads/run/run-123",
    )

    # Verify edge was emitted with knowledge: prefix
    mock_append.assert_called_once()
    edge_kwargs = mock_append.call_args[1]
    assert edge_kwargs["edge_label"] == "knowledge:contains"
    assert edge_kwargs["out_id"] == root.cas_id
    assert edge_kwargs["in_id"] == child.cas_id

    # 4. Record provenance with fragment_id
    import uuid
    artifact_id = str(uuid.uuid4())

    record_provenance_event(
        cx,
        artifact_id=artifact_id,
        event_type="Created",
        fragment_id=root.cas_id,
        operation_id="ingest_001",
    )

    # Verify cursor.execute was called for the two membership writes and provenance write
    assert cursor.execute.call_count == 3


@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_dedup_same_fragment(mock_require):
    """Storing same CAS fragment twice doesn't error (ON CONFLICT DO NOTHING)."""
    from modelado.ikam_graph_repository import store_fragment
    from ikam.forja.cas import cas_fragment

    cx = MagicMock()
    cursor = MagicMock()
    cx.cursor.return_value.__enter__ = MagicMock(return_value=cursor)
    cx.cursor.return_value.__exit__ = MagicMock(return_value=False)

    frag = cas_fragment({"text": "same"}, "text/ikam-paragraph")

    # Store twice — should not raise
    store_fragment(cx, frag, project_id="proj1", ref="refs/heads/main")
    store_fragment(cx, frag, project_id="proj1", ref="refs/heads/main")

    # Both INSERT calls include ON CONFLICT DO NOTHING
    assert cursor.execute.call_count == 2
    for c in cursor.execute.call_args_list:
        assert "ON CONFLICT" in c[0][0]


def test_store_fragment_uses_ref_membership_without_duplicate_payload_bytes(db_connection):
    """Ref membership rows multiply while CAS payload storage stays deduplicated."""
    from ikam.forja.cas import cas_fragment
    from modelado.ikam_graph_repository import insert_fragment, store_fragment
    from ikam.graph import StoredFragment
    import json

    value = {"text": "shared"}
    frag = cas_fragment(value, "application/json")
    payload = json.dumps(value, sort_keys=True, ensure_ascii=False).encode("utf-8")

    insert_fragment(
        db_connection,
        StoredFragment(id=frag.cas_id, mime_type=frag.mime_type, size=len(payload), bytes=payload),
    )
    store_fragment(db_connection, frag, project_id="proj1", ref="refs/heads/main")
    store_fragment(db_connection, frag, project_id="proj1", ref="refs/heads/run/run-123")

    with db_connection.cursor() as cur:
        cur.execute(
            "SELECT ref, value FROM ikam_fragment_store WHERE cas_id = %s ORDER BY ref",
            (frag.cas_id,),
        )
        rows = cur.fetchall()
        cur.execute("SELECT COUNT(*) AS count FROM ikam_fragments WHERE id = %s", (frag.cas_id,))
        payload_count = cur.fetchone()["count"]

    assert [row["ref"] for row in rows] == ["refs/heads/main", "refs/heads/run/run-123"]
    assert all(row["value"] == {"text": "shared"} for row in rows)
    assert payload_count == 1


@patch("modelado.ikam_graph_repository.insert_fragment")
@patch("modelado.ikam_graph_repository.ikam_adapters.v3_to_storage")
@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_insert_domain_fragment_uses_public_adapter_storage_path(
    mock_require,
    mock_v3_to_storage,
    mock_insert_fragment,
):
    """Domain fragment persistence should go through ikam.adapters.v3_to_storage."""
    from ikam.fragments import Fragment
    from ikam.graph import StoredFragment
    from modelado.ikam_graph_repository import insert_domain_fragment

    cx = MagicMock()
    fragment = Fragment(cas_id="cas-test", value={"text": "adapter path"}, mime_type="application/json")
    storage_fragment = StoredFragment.from_bytes(
        b'{"mime_type": "application/json", "value": {"text": "adapter path"}}',
        mime_type="application/json",
    )
    mock_v3_to_storage.return_value = storage_fragment

    insert_domain_fragment(cx, fragment)

    mock_v3_to_storage.assert_called_once_with(fragment)
    mock_insert_fragment.assert_called_once_with(cx, storage_fragment)


def test_create_ikam_schema_adds_ref_scoped_fragment_store(db_connection):
    """The real schema exposes ref-based fragment membership columns and indexes."""
    with db_connection.cursor() as cur:
        cur.execute(
            """
            SELECT column_name
            FROM information_schema.columns
            WHERE table_name = 'ikam_fragment_store'
              AND column_name IN ('ref', 'env')
            ORDER BY column_name
            """
        )
        columns = [row["column_name"] for row in cur.fetchall()]

        cur.execute(
            """
            SELECT indexname
            FROM pg_indexes
            WHERE tablename = 'ikam_fragment_store'
              AND indexname = 'idx_fragment_store_ref'
            """
        )
        index_row = cur.fetchone()

    assert "ref" in columns
    assert index_row is not None


@patch("modelado.ikam_graph_repository._require_ikam_write")
def test_invalid_promotion_does_not_execute(mock_require):
    """Invalid promotion path raises before hitting the database."""
    from modelado.ikam_graph_repository import promote_fragment

    cx = MagicMock()
    cursor = MagicMock()
    cx.cursor.return_value.__enter__ = MagicMock(return_value=cursor)
    cx.cursor.return_value.__exit__ = MagicMock(return_value=False)

    with pytest.raises(TypeError):
        promote_fragment(cx, "cas_abc", from_env="dev", to_env="committed")

    # No database call should have been made
    cursor.execute.assert_not_called()
