import json
import uuid

import pytest

import modelado.fragment_relation_queries as fragment_relation_queries
from modelado.fragment_relation_queries import FragmentRelationQuery


def test_fragment_relation_queries_module_is_explicitly_legacy_only():
    module_doc = fragment_relation_queries.__doc__

    assert module_doc is not None
    assert "legacy" in module_doc.lower()
    assert "compatibility" in module_doc.lower()
    assert "v3" in module_doc.lower()
    assert "does not implement the v3 fragment boundary" in module_doc.lower()


class _TupleCursor:
    def __init__(self, row):
        self._row = row

    def execute(self, query, params=None):
        return self

    def fetchone(self):
        return self._row

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc, tb):
        return False


class _LegacyConnectionWithoutTables:
    def cursor(self):
        return _TupleCursor((1,))

    def execute(self, query, params=None):
        raise AssertionError("legacy relation query should return early when tables are unavailable")


def test_list_relations_returns_empty_when_legacy_tables_are_unavailable_with_tuple_rows():
    q = FragmentRelationQuery(_LegacyConnectionWithoutTables())

    assert q.list_relations(project_id="proj-1") == []


def _skip_if_legacy_fragment_tables_missing(db_connection) -> None:
    """Skip tests that require legacy derived fragment tables.

    Relation query tests currently depend on the legacy mutable tables
    (ikam_fragment_meta/content/radicals). Under the vNext schema cutover these
    tables are deleted in favor of immutable fragment objects.
    """

    required = ("ikam_fragment_meta", "ikam_fragment_content")
    with db_connection.cursor() as cur:
        cur.execute(
            """
            SELECT table_name
            FROM information_schema.tables
            WHERE table_schema = current_schema()
              AND table_name = ANY(%s)
            """,
            (list(required),),
        )
        present = {row[0] for row in cur.fetchall()}

    missing = [t for t in required if t not in present]
    if missing:
        pytest.skip(
            "Legacy fragment tables not present (vNext schema cutover): "
            + ", ".join(missing)
        )


def _ensure_container_artifact(db_connection, artifact_id: uuid.UUID) -> None:
    with db_connection.cursor() as cur:
        cur.execute(
            """
            INSERT INTO ikam_artifacts (id, kind, title)
            VALUES (%s, 'document', 'Test Artifact')
            ON CONFLICT (id) DO NOTHING
            """,
            (artifact_id,),
        )


def _insert_relation_fragment(
    db_connection,
    *,
    artifact_id: uuid.UUID,
    fragment_id: str,
    predicate: str,
    subject_ids: list[str],
    object_ids: list[str],
    directed: bool = True,
    confidence_score: float = 0.9,
    salience: float = 0.5,
) -> None:
    with db_connection.cursor() as cur:
        cur.execute(
            """
            INSERT INTO ikam_fragments (id, mime_type, size, bytes)
            VALUES (%s, 'application/vnd.narraciones.ikam-relation+json', %s, %s)
            ON CONFLICT (id) DO NOTHING
            """,
            (fragment_id, 2, b"{}"),
        )
        cur.execute(
            """
            INSERT INTO ikam_fragment_meta (fragment_id, artifact_id, level, type, salience, created_at, updated_at)
            VALUES (%s, %s, 0, 'relation', %s, now(), now())
            ON CONFLICT (fragment_id) DO NOTHING
            """,
            (fragment_id, artifact_id, salience),
        )
        cur.execute(
            """
            INSERT INTO ikam_fragment_content (fragment_id, content_type, content)
            VALUES (%s, 'relation', CAST(%s AS JSONB))
            ON CONFLICT (fragment_id) DO NOTHING
            """,
            (
                fragment_id,
                json.dumps(
                    {
                        "predicate": predicate,
                        "subject_fragment_ids": subject_ids,
                        "object_fragment_ids": object_ids,
                        "directed": directed,
                        "confidence_score": confidence_score,
                        "qualifiers": {"source": "test"},
                    }
                ),
            ),
        )


@pytest.mark.integration
def test_relation_query_filters_by_predicate_and_endpoints(db_connection):
    _skip_if_legacy_fragment_tables_missing(db_connection)

    container_artifact_id = uuid.uuid4()
    _ensure_container_artifact(db_connection, container_artifact_id)

    a = "a-" + uuid.uuid4().hex
    b = "b-" + uuid.uuid4().hex
    c = "c-" + uuid.uuid4().hex

    rel1 = "rel-" + uuid.uuid4().hex
    rel2 = "rel-" + uuid.uuid4().hex

    _insert_relation_fragment(
        db_connection,
        artifact_id=container_artifact_id,
        fragment_id=rel1,
        predicate="depends_on",
        subject_ids=[a],
        object_ids=[b],
    )
    _insert_relation_fragment(
        db_connection,
        artifact_id=container_artifact_id,
        fragment_id=rel2,
        predicate="is_composed_of",
        subject_ids=[a],
        object_ids=[c],
    )

    q = FragmentRelationQuery(db_connection)

    all_edges = q.list_relations(project_id=container_artifact_id)
    assert {e["id"] for e in all_edges} == {rel1, rel2}

    depends = q.list_relations(project_id=container_artifact_id, predicate="depends_on")
    assert [e["id"] for e in depends] == [rel1]

    for_subject = q.for_subject(a, project_id=container_artifact_id)
    assert {e["id"] for e in for_subject} == {rel1, rel2}

    for_object = q.for_object(c, project_id=container_artifact_id)
    assert [e["id"] for e in for_object] == [rel2]


@pytest.mark.integration
def test_relation_query_filters_by_subject_and_object(db_connection):
    _skip_if_legacy_fragment_tables_missing(db_connection)

    container_artifact_id = uuid.uuid4()
    _ensure_container_artifact(db_connection, container_artifact_id)

    s1 = "s1-" + uuid.uuid4().hex
    s2 = "s2-" + uuid.uuid4().hex
    o1 = "o1-" + uuid.uuid4().hex
    o2 = "o2-" + uuid.uuid4().hex

    rel = "rel-" + uuid.uuid4().hex
    _insert_relation_fragment(
        db_connection,
        artifact_id=container_artifact_id,
        fragment_id=rel,
        predicate="related_to",
        subject_ids=[s1, s2],
        object_ids=[o1, o2],
    )

    q = FragmentRelationQuery(db_connection)

    assert [e["id"] for e in q.list_relations(container_artifact_id, subject_fragment_id=s2)] == [rel]
    assert [e["id"] for e in q.list_relations(container_artifact_id, object_fragment_id=o2)] == [rel]
    assert (
        q.list_relations(
            container_artifact_id,
            predicate="related_to",
            subject_fragment_id=s1,
            object_fragment_id=o1,
        )[0]["id"]
        == rel
    )
