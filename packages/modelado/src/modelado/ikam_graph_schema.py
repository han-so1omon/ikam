"""Schema creation for IKAM graph tables.

This module provides functions to create and drop IKAM graph tables for testing.
Uses production table names and column definitions (ikam_artifacts, ikam_fragments,
ikam_fragment_objects, etc.) to match the authoritative DDL in ``db.py``.

Column semantics aligned with:
- ``docs/ikam/IKAM_FRAGMENT_ALGEBRA_V3.md`` §2.3 (Artifact.root_fragment_id)
- ``docs/ikam/IKAM_MONOID_ALGEBRA_CONTRACT.md`` §4.B4 (append-only versioning)
- Production DDL in ``modelado.db.ensure_ikam_schema()``
"""

from typing import Any
import psycopg


def create_ikam_schema(cx: psycopg.Connection[Any]) -> None:
    """Create all IKAM graph tables.

    Args:
        cx: Database connection
    """
    with cx.cursor() as cur:
        # Fragments table (CAS storage)
        cur.execute(
            """
            CREATE TABLE IF NOT EXISTS ikam_fragments (
                id TEXT PRIMARY KEY,
                mime_type TEXT NOT NULL,
                size BIGINT NOT NULL,
                bytes BYTEA NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                CONSTRAINT chk_fragments_size_positive CHECK (size > 0)
            )
            """
        )
        cur.execute("CREATE INDEX IF NOT EXISTS idx_fragments_mime ON ikam_fragments(mime_type)")

        # Staging Fragments table (Session-scoped CAS storage)
        cur.execute(
            """
            CREATE TABLE IF NOT EXISTS ikam_staging_fragments (
                id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                mime_type TEXT NOT NULL,
                size BIGINT NOT NULL,
                bytes BYTEA NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                PRIMARY KEY (session_id, id),
                CONSTRAINT chk_staging_fragments_size_positive CHECK (size > 0)
            )
            """
        )
        cur.execute("CREATE INDEX IF NOT EXISTS idx_staging_fragments_id ON ikam_staging_fragments(id)")

        # Artifacts table — V3: root_fragment_id is the DAG reconstruction entrypoint
        cur.execute(
            """
            CREATE TABLE IF NOT EXISTS ikam_artifacts (
                id UUID PRIMARY KEY,
                kind TEXT NOT NULL,
                title TEXT,
                project_id TEXT,
                status TEXT,
                root_fragment_id TEXT,
                head_object_id TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ DEFAULT NOW()
            )
            """
        )
        # Idempotent column additions for pre-existing tables
        for col, typedef in [
            ("project_id", "TEXT"),
            ("root_fragment_id", "TEXT"),
            ("head_object_id", "TEXT"),
        ]:
            cur.execute(f"ALTER TABLE ikam_artifacts ADD COLUMN IF NOT EXISTS {col} {typedef}")  # type: ignore
        cur.execute("CREATE INDEX IF NOT EXISTS idx_artifacts_kind ON ikam_artifacts(kind)")
        cur.execute("CREATE INDEX IF NOT EXISTS idx_artifacts_created ON ikam_artifacts(created_at)")
        cur.execute("CREATE INDEX IF NOT EXISTS idx_ikam_artifacts_project_id ON ikam_artifacts(project_id)")

        # Artifact-Fragment associations (legacy positional references)
        cur.execute(
            """
            CREATE TABLE IF NOT EXISTS ikam_artifact_fragments (
                artifact_id UUID NOT NULL REFERENCES ikam_artifacts(id) ON DELETE CASCADE,
                fragment_id TEXT NOT NULL REFERENCES ikam_fragments(id) ON DELETE RESTRICT,
                position INT NOT NULL,
                PRIMARY KEY (artifact_id, fragment_id),
                CONSTRAINT chk_artifact_fragments_position_nonnegative CHECK (position >= 0)
            )
            """
        )
        cur.execute("CREATE INDEX IF NOT EXISTS idx_artifact_fragments_frag ON ikam_artifact_fragments(fragment_id)")

        # Fragment object manifests (immutable, CAS-addressed)
        # Production schema: (object_id, root_fragment_id, manifest, created_at, created_by)
        cur.execute(
            """
            CREATE TABLE IF NOT EXISTS ikam_fragment_objects (
                object_id TEXT PRIMARY KEY,
                root_fragment_id TEXT NOT NULL REFERENCES ikam_fragments(id) ON DELETE RESTRICT,
                manifest JSONB NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                created_by UUID
            )
            """
        )
        cur.execute("CREATE INDEX IF NOT EXISTS idx_ikam_fragment_objects_object_id ON ikam_fragment_objects(object_id)")
        cur.execute("CREATE INDEX IF NOT EXISTS idx_ikam_fragment_objects_root_fragment_id ON ikam_fragment_objects(root_fragment_id)")

        # head_object_id FK (idempotent)
        cur.execute(
            """
            DO $$
            BEGIN
              IF NOT EXISTS (
                SELECT 1 FROM pg_constraint WHERE conname = 'ikam_artifacts_head_object_id_fkey'
              ) THEN
                ALTER TABLE ikam_artifacts
                  ADD CONSTRAINT ikam_artifacts_head_object_id_fkey
                  FOREIGN KEY (head_object_id)
                  REFERENCES ikam_fragment_objects(object_id)
                  ON DELETE RESTRICT;
              END IF;
            END $$
            """
        )
        cur.execute("CREATE INDEX IF NOT EXISTS idx_ikam_artifacts_head_object_id ON ikam_artifacts(head_object_id)")

        # Provenance events table (append-only event log for collaboration metadata)
        cur.execute(
            """
            CREATE TABLE IF NOT EXISTS ikam_provenance_events (
                id UUID PRIMARY KEY,
                artifact_id UUID REFERENCES ikam_artifacts(id) ON DELETE CASCADE,
                -- NOTE: derivation_id is a logical identifier only. The canonical derivation
                -- relationships are tracked via graph_edge_events; do not enforce an FK.
                derivation_id UUID,
                event_type TEXT NOT NULL, -- Created | Modified | Derived | Rendered | SystemMutated
                author_id UUID,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                details JSONB
            )
            """
        )
        cur.execute("CREATE INDEX IF NOT EXISTS idx_prov_events_artifact ON ikam_provenance_events(artifact_id)")
        cur.execute("CREATE INDEX IF NOT EXISTS idx_prov_events_derivation ON ikam_provenance_events(derivation_id)")
        cur.execute("CREATE INDEX IF NOT EXISTS idx_prov_events_author ON ikam_provenance_events(author_id)")
        cur.execute("CREATE INDEX IF NOT EXISTS idx_prov_events_type_created ON ikam_provenance_events(event_type, created_at)")

        # --- Compression/Re-Render Pipeline Tables ---
        cur.execute(IKAM_FRAGMENT_STORE_DDL)
        cur.execute(IKAM_NORMALIZATION_STATS_DDL)
        cur.execute(IKAM_PROVENANCE_ALTER_DDL)


def drop_legacy_tables(cx: psycopg.Connection[Any]) -> None:
    """Drop legacy tables superseded by Fragment+Connection graph."""
    with cx.cursor() as cur:
        cur.execute("DROP TABLE IF EXISTS ikam_artifact_fragments CASCADE;")
        cur.execute("DROP TABLE IF EXISTS ikam_relation_overlay_relations CASCADE;")
        cur.execute("DROP TABLE IF EXISTS ikam_relation_overlays CASCADE;")
        cur.execute("DROP TABLE IF EXISTS graph_edge_projection_checkpoints CASCADE;")


def drop_ikam_schema(cx: psycopg.Connection[Any]) -> None:
    """Drop all IKAM graph tables.

    Args:
        cx: Database connection
    """
    with cx.cursor() as cur:
        cur.execute("DROP TABLE IF EXISTS ikam_provenance_events CASCADE")
        cur.execute("DROP TABLE IF EXISTS ikam_fragment_objects CASCADE")
        cur.execute("DROP TABLE IF EXISTS ikam_artifact_fragments CASCADE")
        cur.execute("DROP TABLE IF EXISTS ikam_artifacts CASCADE")
        cur.execute("DROP TABLE IF EXISTS ikam_fragments CASCADE")
        cur.execute("DROP MATERIALIZED VIEW IF EXISTS ikam_artifact_edge_counts CASCADE")


# -- Compression/Re-Render Pipeline DDL --

IKAM_FRAGMENT_STORE_DDL = """
CREATE TABLE IF NOT EXISTS ikam_fragment_store (
    cas_id TEXT NOT NULL,
    ref TEXT,
    env TEXT,
    operation_id TEXT,
    project_id TEXT NOT NULL,
    value JSONB,
    mime_type TEXT,
    embedding VECTOR(768),
    structure_hash TEXT,
    embedding_model TEXT,
    created_at TIMESTAMPTZ DEFAULT now()
);

ALTER TABLE ikam_fragment_store ADD COLUMN IF NOT EXISTS ref TEXT;
ALTER TABLE ikam_fragment_store ADD COLUMN IF NOT EXISTS env TEXT;

UPDATE ikam_fragment_store
SET ref = CASE
    WHEN ref IS NOT NULL THEN ref
    WHEN env = 'committed' THEN 'refs/heads/main'
    WHEN env = 'staging' THEN 'refs/heads/staging/default'
    WHEN env = 'dev' THEN 'refs/heads/run/default'
    ELSE 'refs/heads/main'
END
WHERE ref IS NULL;

ALTER TABLE ikam_fragment_store ALTER COLUMN ref SET DEFAULT 'refs/heads/main';
UPDATE ikam_fragment_store SET ref = 'refs/heads/main' WHERE ref IS NULL;
ALTER TABLE ikam_fragment_store ALTER COLUMN ref SET NOT NULL;

DROP INDEX IF EXISTS uq_fragment_store_pk;
CREATE UNIQUE INDEX IF NOT EXISTS uq_fragment_store_pk ON ikam_fragment_store(cas_id, ref, COALESCE(operation_id, ''));

CREATE INDEX IF NOT EXISTS idx_fragment_store_project ON ikam_fragment_store(project_id);
CREATE INDEX IF NOT EXISTS idx_fragment_store_ref ON ikam_fragment_store(ref);
CREATE INDEX IF NOT EXISTS idx_fragment_store_mime ON ikam_fragment_store(mime_type);
CREATE INDEX IF NOT EXISTS idx_fragment_store_structure ON ikam_fragment_store(structure_hash);
CREATE INDEX IF NOT EXISTS idx_fragment_store_embedding ON ikam_fragment_store USING hnsw (embedding vector_cosine_ops);
"""

IKAM_NORMALIZATION_STATS_DDL = """
CREATE TABLE IF NOT EXISTS ikam_normalization_stats (
    id SERIAL PRIMARY KEY,
    project_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    family TEXT NOT NULL,
    candidates_found INT DEFAULT 0,
    candidates_verified INT DEFAULT 0,
    candidates_accepted INT DEFAULT 0,
    storage_saved_bytes BIGINT DEFAULT 0,
    avg_verification_ms FLOAT,
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_norm_stats_project ON ikam_normalization_stats(project_id);
CREATE INDEX IF NOT EXISTS idx_norm_stats_family ON ikam_normalization_stats(family);
"""

IKAM_PROVENANCE_ALTER_DDL = """
ALTER TABLE ikam_provenance_events
    ADD COLUMN IF NOT EXISTS fragment_id TEXT,
    ADD COLUMN IF NOT EXISTS operation_id TEXT;

CREATE INDEX IF NOT EXISTS idx_prov_events_fragment ON ikam_provenance_events(fragment_id);
CREATE INDEX IF NOT EXISTS idx_prov_events_operation ON ikam_provenance_events(operation_id);
"""


def truncate_ikam_tables(cx: psycopg.Connection[Any]) -> None:
    """Truncate all IKAM tables (preserves schema).

    Args:
        cx: Database connection
    """
    with cx.cursor() as cur:
        cur.execute(
            """
            TRUNCATE ikam_fragments, ikam_artifacts, ikam_artifact_fragments,
                     ikam_fragment_objects, ikam_provenance_events CASCADE
            """
        )
