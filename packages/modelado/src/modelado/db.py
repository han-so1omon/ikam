"""Database utilities for modelado framework.

Provides connection pooling, schema management, and database utilities.
Designed to be framework-agnostic with minimal dependencies.
"""

from __future__ import annotations

import contextvars
import logging
import os
import threading
import time
from contextlib import contextmanager
from typing import Generator, Iterable, Optional, Sequence, Protocol, runtime_checkable, cast

import psycopg
from psycopg.abc import Query
from psycopg import sql as psycopg_sql
from psycopg.rows import dict_row
from psycopg_pool import ConnectionPool

logger = logging.getLogger(__name__)


_LEGACY_IKAM_FRAGMENT_TABLES = (
    "ikam_fragment_meta",
    "ikam_fragment_content",
    "ikam_fragment_radicals",
)


def _fail_if_legacy_ikam_fragment_tables_exist(cx: "ConnectionWrapper") -> None:
    """Hard-fail if legacy mutable IKAM fragment tables exist.

    The vNext schema requires immutable fragment objects; databases containing the
    legacy derived tables must be migrated/dropped before the service can start.
    """

    rows = cx.execute(
        """
        SELECT c.relname
        FROM pg_catalog.pg_class c
        JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = current_schema()
          AND c.relkind IN ('r', 'p')
          AND c.relname = ANY(%s)
        ORDER BY c.relname
        """,
        (list(_LEGACY_IKAM_FRAGMENT_TABLES),),
    ).fetchall()
    if not rows:
        return

    def _relname(row) -> str:
        try:
            return str(row["relname"])
        except Exception:
            try:
                return str(row[0])
            except Exception:
                return str(row)

    legacy = ", ".join([_relname(row) for row in rows])
    raise RuntimeError(
        "Legacy IKAM fragment tables detected (mutable derived tables): "
        f"{legacy}. "
        "Drop these tables before startup to proceed with the immutable-head schema cutover."
    )

_pool: ConnectionPool | None = None
_schema_ready = False
_last_pytest_test: str | None = None
_schema_lock = threading.Lock()

# Track connection lifecycle for debugging hangs
_connection_counter = 0
_connection_counter_lock = threading.Lock()

# Active connection context for re-entrant get_connection() support
_current_connection: contextvars.ContextVar[Optional["ConnectionWrapper"]] = (
    contextvars.ContextVar("modelado_current_connection", default=None)
)


def _ensure_fragment_store_ref_scope(cx: "ConnectionWrapper") -> None:
    cx.execute("ALTER TABLE ikam_fragment_store ADD COLUMN IF NOT EXISTS ref TEXT")
    cx.execute("ALTER TABLE ikam_fragment_store ADD COLUMN IF NOT EXISTS env TEXT")
    cx.execute(
        """
        UPDATE ikam_fragment_store
        SET ref = CASE
            WHEN ref IS NOT NULL THEN ref
            WHEN env = 'committed' THEN 'refs/heads/main'
            WHEN env = 'staging' THEN 'refs/heads/staging/default'
            WHEN env = 'dev' THEN 'refs/heads/run/default'
            ELSE 'refs/heads/main'
        END
        WHERE ref IS NULL
        """
    )
    cx.execute("ALTER TABLE ikam_fragment_store ALTER COLUMN ref SET DEFAULT 'refs/heads/main'")
    cx.execute("UPDATE ikam_fragment_store SET ref = 'refs/heads/main' WHERE ref IS NULL")
    cx.execute("ALTER TABLE ikam_fragment_store ALTER COLUMN ref SET NOT NULL")
    cx.execute("DROP INDEX IF EXISTS uq_fragment_store_pk")
    cx.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS uq_fragment_store_pk ON ikam_fragment_store(cas_id, ref, COALESCE(operation_id, ''))"
    )
    cx.execute("CREATE INDEX IF NOT EXISTS idx_fragment_store_ref ON ikam_fragment_store(ref)")


def _get_connection_id() -> str:
    """Generate unique connection ID for debugging."""
    global _connection_counter
    with _connection_counter_lock:
        _connection_counter += 1
        return f"conn-{_connection_counter}"


def _configure_connection(conn: psycopg.Connection) -> None:
    """Configure connection on checkout from pool to ensure clean state.
    
    This callback runs every time a connection is retrieved from the pool,
    ensuring any stale transaction state from thread interruption is cleared.
    
    CRITICAL: Must leave connection in IDLE state (no open transaction).
    """
    pid = conn.info.backend_pid
    logger.debug("[Pool] Configuring connection PID=%s", pid)
    
    try:
        # Check if connection is in a transaction and roll it back
        if conn.info.transaction_status != psycopg.pq.TransactionStatus.IDLE:
            logger.warning(
                "[Pool] Connection PID=%s retrieved from pool still in transaction (status: %s), rolling back",
                pid, conn.info.transaction_status,
            )
            conn.rollback()
        
        # Verify connection is healthy with a simple query, then commit to return to IDLE
        conn.execute("SELECT 1")
        conn.commit()  # CRITICAL: Return to IDLE state after validation query
        logger.debug("[Pool] Connection PID=%s validated and ready", pid)
    except Exception as err:
        logger.error("[Pool] Connection PID=%s validation failed during pool checkout: %s", pid, err, exc_info=True)
        # Ensure connection is in IDLE state even on error
        try:
            conn.rollback()
        except Exception:
            pass
        # Let psycopg3 handle the bad connection (it will discard and retry)
        raise


def _get_pool() -> ConnectionPool:
    global _pool
    if _pool is None:
        # Database URL from environment with fallback
        database_url = os.getenv(
            "DATABASE_URL",
            os.getenv(
                "NARRACIONES_DB",
                "postgresql://narraciones:narraciones@localhost:5432/narraciones"
            )
        )
        max_size = int(
          os.getenv("NARRACIONES_DB_POOL_MAX")
          or os.getenv("DCF_STORYTELLER_DB_POOL_MAX", "30")
        )
        min_size = int(os.getenv("NARRACIONES_DB_POOL_MIN", "5"))
        if min_size > max_size:
          min_size = max_size
        # Connection acquisition timeout in seconds (default 30s)
        timeout = float(os.getenv("NARRACIONES_DB_POOL_TIMEOUT", "30"))
        # Configure connection lifecycle to prevent idle-in-transaction leaks:
        # - max_idle: recycle connections idle for >10min (600s)
        # - max_lifetime: force-close connections older than 30min (1800s)
        # - configure: validate connections on checkout with ROLLBACK to clear any stale transactions
        # Apply per-connection safety timeouts at session start (not per request)
        # to avoid extra round-trips during hot-path operations.
        pool_options = "-c lock_timeout=10s -c statement_timeout=120s -c idle_in_transaction_session_timeout=30s"

        _pool = ConnectionPool(
            conninfo=database_url,
          min_size=min_size,
            max_size=max_size,
            timeout=timeout,
            max_idle=600.0,  # Recycle connections idle >10min
            max_lifetime=1800.0,  # Force close connections >30min old
          kwargs={"row_factory": dict_row, "options": pool_options},
            configure=_configure_connection,  # Validate and reset connection state on checkout
            open=True,  # Explicitly open pool (future psycopg default will be False)
        )
    return _pool


def _convert_query(sql: str) -> str:
    """Convert SQLite-style '?' placeholders to PostgreSQL '%s'."""

    result: list[str] = []
    in_single = False
    in_double = False
    prev_char = ""
    for ch in sql:
        if ch == "'" and not in_double and prev_char != "\\":
            in_single = not in_single
        elif ch == '"' and not in_single and prev_char != "\\":
            in_double = not in_double
        if ch == "?" and not in_single and not in_double:
            result.append("%s")
        else:
            result.append(ch)
        prev_char = ch
    return "".join(result)


def _convert_params(params: Sequence | None) -> Sequence | None:
    if params is None:
        return None
    return params


def reset_pool_for_pytest() -> None:
    """Close the shared connection pool so pytest can swap databases safely."""

    global _pool, _schema_ready, _last_pytest_test
    if _pool is not None:
        try:
            _pool.close()
        except Exception:  # pragma: no cover - defensive guard for local dev
            logger.warning("Failed closing connection pool during pytest reset", exc_info=True)
        finally:
            _pool = None
    _schema_ready = False
    _last_pytest_test = None


class ConnectionWrapper:
    def __init__(self, conn: psycopg.Connection):
        self._conn = conn

    def execute(self, sql: str | psycopg_sql.Composable, params: Sequence | None = None):
        converted_params = _convert_params(params)
        if isinstance(sql, str):
            converted_sql = _convert_query(sql)
            return self._conn.execute(cast(Query, converted_sql), converted_params)
        return self._conn.execute(cast(Query, sql), converted_params)

    def executemany(self, sql: str | psycopg_sql.Composable, params_seq: Iterable[Sequence]):
        with self._conn.cursor(row_factory=dict_row) as cur:
            if isinstance(sql, str):
                converted_sql = _convert_query(sql)
                cur.executemany(cast(Query, converted_sql), params_seq)
            else:
                cur.executemany(cast(Query, sql), params_seq)
            return cur

    def cursor(self, *args, **kwargs):
        if "row_factory" not in kwargs:
            kwargs["row_factory"] = dict_row
        return self._conn.cursor(*args, **kwargs)

    def commit(self) -> None:
        self._conn.commit()

    def rollback(self) -> None:
        self._conn.rollback()

    def __getattr__(self, item):
        return getattr(self._conn, item)


def _maybe_reset_for_pytest(conn: ConnectionWrapper) -> None:
    global _last_pytest_test
    if not _schema_ready:
        return
    current = os.environ.get("PYTEST_CURRENT_TEST")
    if not current:
        return
    ident = current.split(" (")[0]
    if ident == _last_pytest_test:
        return
    _last_pytest_test = ident
    # Database reset is handled by pytest fixtures in tests.conftest


@contextmanager
def connection_scope() -> Generator[ConnectionWrapper, None, None]:
    conn_id = _get_connection_id()
    pool = _get_pool()
    acquire_start = time.time()
    logger.debug("[%s] Acquiring connection from pool...", conn_id)
    
    with pool.connection() as raw_conn:
        acquire_time = time.time() - acquire_start
        pid = raw_conn.info.backend_pid
        logger.debug("[%s] Acquired connection PID=%s in %.3fs", conn_id, pid, acquire_time)
        
        if acquire_time > 1.0:
            logger.warning(
                "[%s] Slow connection acquisition: %.3fs (pool may be exhausted or under load)",
                conn_id, acquire_time
            )
        
        # Per-connection safety timeouts are configured once at connection creation
        # via psycopg "options" in the pool.
        
        wrapper = ConnectionWrapper(raw_conn)
        token = _current_connection.set(wrapper)
        scope_start = time.time()
        
        try:
            _maybe_reset_for_pytest(wrapper)
            logger.debug("[%s] Yielding connection to caller", conn_id)
            yield wrapper
            
            commit_start = time.time()
            raw_conn.commit()
            commit_time = time.time() - commit_start
            scope_time = time.time() - scope_start
            
            logger.debug(
                "[%s] Transaction committed in %.3fs (total scope: %.3fs)",
                conn_id, commit_time, scope_time
            )
            
            if scope_time > 5.0:
                logger.warning(
                    "[%s] Long-running transaction: %.3fs (consider breaking into smaller operations)",
                    conn_id, scope_time
                )
                
        except psycopg.errors.LockNotAvailable as lock_err:
            # Handle lock timeout with detailed diagnostics
            raw_conn.rollback()
            blocker_info = _get_blocking_info(raw_conn)
            scope_time = time.time() - scope_start
            
            logger.error(
                "[%s] Lock timeout after %.3fs. Transaction rolled back. Blocking processes: %s",
                conn_id, scope_time, blocker_info,
                exc_info=True,
            )
            # Re-raise with context for caller
            raise RuntimeError(
                f"Database lock timeout. Blocking PIDs: {blocker_info}. "
                "Operation rolled back. Check for long-running transactions or contention."
            ) from lock_err
            
        except psycopg.errors.QueryCanceled as cancel_err:
            raw_conn.rollback()
            scope_time = time.time() - scope_start
            logger.error(
                "[%s] Statement timeout after %.3fs. Transaction rolled back.",
                conn_id, scope_time,
                exc_info=True
            )
            raise RuntimeError(
                "Database statement timeout. Operation rolled back. "
                "Query may be too expensive or database is overloaded."
            ) from cancel_err
            
        except Exception:
            scope_time = time.time() - scope_start
            logger.error(
                "[%s] Exception during transaction (%.3fs elapsed), rolling back",
                conn_id, scope_time,
                exc_info=True
            )
            raw_conn.rollback()
            raise
            
        finally:
            # Clear context variable for re-entrant support
            _current_connection.reset(token)
            # Ensure connection is always closed and returned to pool
            # even if there's an exception during yield or commit
            try:
                if raw_conn.info.transaction_status != psycopg.pq.TransactionStatus.IDLE:
                    # If transaction is still active (shouldn't happen, but defensive)
                    logger.warning(
                        "[%s] Connection PID=%s still in transaction after scope exit (status: %s), rolling back",
                        conn_id, pid, raw_conn.info.transaction_status
                    )
                    raw_conn.rollback()
                
                total_time = time.time() - acquire_start
                logger.debug(
                    "[%s] Returning connection PID=%s to pool (total lifecycle: %.3fs)",
                    conn_id, pid, total_time
                )
            except Exception as cleanup_err:  # pragma: no cover
                logger.error(
                    "[%s] Error during connection cleanup: %s",
                    conn_id, cleanup_err,
                    exc_info=True
                )


def _get_blocking_info(conn: psycopg.Connection) -> str:
    """Query pg_blocking_pids() to identify processes holding locks."""
    try:
        # Use a fresh connection or the same one (if still valid) to query blocker info
        result = conn.execute(
            """
            SELECT pid, query, state, query_start
            FROM pg_stat_activity
            WHERE pid = ANY(pg_blocking_pids(pg_backend_pid()))
            LIMIT 5
            """
        ).fetchall()
        if not result:
            return "No blocking processes found (lock may have been released)"
        blockers = []
        for row in result:
            blockers.append(
                f"PID {row[0]} [{row[2]}] query: {row[1][:100]} (started {row[3]})"
            )
        return "; ".join(blockers)
    except Exception as err:
        logger.warning("Failed to query blocking PIDs: %s", err)
        return f"Unable to retrieve blocker info: {err}"


@runtime_checkable
class ConnectionScope(Protocol):
  def __enter__(self) -> "ConnectionWrapper": ...
  def __exit__(self, typ, value, traceback) -> bool | None: ...


class _ReentrantConnectionScope:
    """No-op context manager that yields an existing connection.
    
    Used when get_connection() is called inside an active connection_scope().
    Prevents nested commits/rollbacks and ensures the outer scope manages the transaction.
    """
    def __init__(self, wrapper: ConnectionWrapper):
        self._wrapper = wrapper
    
    def __enter__(self) -> ConnectionWrapper:
        return self._wrapper
    
    def __exit__(self, typ, value, traceback):
        # Do not commit or rollback; let outer scope handle it
        return False


def get_connection() -> ConnectionScope:
    """Return a connection scope context manager.
    
    Re-entrant behavior:
    - If called inside an active connection_scope(), returns the current connection
      without creating a new transaction scope.
    - Otherwise, creates a new connection_scope() with full transaction semantics.
    
    This allows service methods to be called both standalone and within an existing
    transaction without causing FK visibility issues or nested commit/rollback errors.
    """
    active = _current_connection.get()
    if active is not None:
        return _ReentrantConnectionScope(active)
    return connection_scope()


def ensure_schema(seed_model_inputs: Optional[list[dict]] = None) -> None:
    """Ensure normalized tables exist in PostgreSQL.
    
    Retries on lock timeout to handle startup races when multiple services
    initialize simultaneously.
    
    Args:
        seed_model_inputs: Optional list of default model inputs to seed.
            Each dict should have keys: key, value, notes, kind.
    """
    import time

    global _schema_ready
    if _schema_ready:
        return

    with _schema_lock:
        if _schema_ready:
            return
        
        # Retry logic for startup races (multiple services creating schema simultaneously)
        max_retries = 5
        retry_delay = 0.5  # Start with 500ms
        last_error = None
        
        for attempt in range(max_retries):
            try:
                _ensure_schema_once(seed_model_inputs)
                _schema_ready = True
                return
            except RuntimeError as err:
                # RuntimeError wraps LockNotAvailable from connection_scope()
                last_error = err
                if attempt < max_retries - 1:
                    logger.warning(
                        "Schema creation locked (attempt %d/%d), retrying in %.1fs: %s",
                        attempt + 1, max_retries, retry_delay, err
                    )
                    time.sleep(retry_delay)
                    retry_delay *= 2  # Exponential backoff
                else:
                    logger.error(
                        "Failed to create schema after %d attempts: %s",
                        max_retries, err
                    )
        
        # If we exhausted retries, raise the last error
        raise RuntimeError(
            f"Could not initialize schema after {max_retries} attempts. "
            "Another service may be holding locks."
        ) from last_error


def _ensure_schema_once(seed_model_inputs: Optional[list[dict]] = None) -> None:
    """Execute schema creation (called by ensure_schema with retry logic)."""
    with connection_scope() as cx:
        try:
            cx.execute("CREATE EXTENSION IF NOT EXISTS vector")
        except (psycopg.errors.UniqueViolation, psycopg.errors.DuplicateObject) as exc:
            logger.debug("Vector extension already exists, continuing without error: %s", exc)
        except psycopg.Error:
            logger.exception("Failed ensuring vector extension")
            raise

        _fail_if_legacy_ikam_fragment_tables_exist(cx)

        # Drop legacy type if present without table (migration guard)
        cx.execute(
            """
            DO $$
            BEGIN
              IF EXISTS (
                SELECT 1
                FROM pg_type t
                JOIN pg_namespace n ON n.oid = t.typnamespace
                WHERE t.typname = 'notion_sync_queue'
                  AND n.nspname = current_schema()
              )
              AND NOT EXISTS (
                SELECT 1
                FROM pg_class c
                JOIN pg_namespace n ON n.oid = c.relnamespace
                WHERE c.relname = 'notion_sync_queue'
                  AND c.relkind IN ('r', 'p')
                  AND n.nspname = current_schema()
              ) THEN
                EXECUTE 'DROP TYPE notion_sync_queue';
              END IF;
            END $$;
            """
        )

        # Core schema
        cx.execute(
            """
            CREATE TABLE IF NOT EXISTS users (
              id TEXT PRIMARY KEY,
              email TEXT UNIQUE,
              name TEXT,
              picture TEXT,
              provider TEXT,
              created_at BIGINT NOT NULL,
              last_seen_at BIGINT NOT NULL,
              extra JSONB NOT NULL DEFAULT '{}'::jsonb
            );

            CREATE TABLE IF NOT EXISTS workspaces (
              id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
              name TEXT NOT NULL,
              slug TEXT NOT NULL UNIQUE,
              type TEXT NOT NULL CHECK (type IN ('personal','organization')),
              created_by UUID NOT NULL,
              created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
              updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            );

            CREATE TABLE IF NOT EXISTS teams (
              id TEXT PRIMARY KEY,
              name TEXT NOT NULL,
              created_at BIGINT NOT NULL,
              created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
              meta JSONB NOT NULL DEFAULT '{}'::jsonb
            );
            CREATE INDEX IF NOT EXISTS idx_teams_created_by ON teams(created_by);
            ALTER TABLE teams ADD COLUMN IF NOT EXISTS workspace_id UUID REFERENCES workspaces(id);
            CREATE INDEX IF NOT EXISTS idx_teams_workspace_id ON teams(workspace_id);

            CREATE TABLE IF NOT EXISTS team_members (
              team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
              user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
              role TEXT NOT NULL CHECK(role IN ('owner','editor','viewer')) DEFAULT 'editor',
              joined_at BIGINT NOT NULL,
              invited_by TEXT REFERENCES users(id) ON DELETE SET NULL,
              PRIMARY KEY(team_id, user_id)
            );
            CREATE INDEX IF NOT EXISTS idx_team_members_user ON team_members(user_id);

            CREATE TABLE IF NOT EXISTS team_invitations (
              id SERIAL PRIMARY KEY,
              team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
              email TEXT NOT NULL,
              status TEXT NOT NULL CHECK(status IN ('pending','accepted','revoked','expired')) DEFAULT 'pending',
              invited_by TEXT REFERENCES users(id) ON DELETE SET NULL,
              invited_at BIGINT NOT NULL,
              responded_at BIGINT
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_team_invitations_unique ON team_invitations(team_id, email);

            CREATE TABLE IF NOT EXISTS model_inputs (
              key   TEXT PRIMARY KEY,
              value DOUBLE PRECISION NOT NULL,
              notes TEXT,
              kind  TEXT CHECK(kind IN ('number','currency','percent')) DEFAULT 'number'
            );

            CREATE TABLE IF NOT EXISTS model_variables (
              key        TEXT PRIMARY KEY,
              value      DOUBLE PRECISION NOT NULL,
              notes      TEXT,
              kind       TEXT CHECK(kind IN ('number','currency','percent')) DEFAULT 'number',
              updated_at BIGINT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS model_formulas (
              id SERIAL PRIMARY KEY,
              name TEXT UNIQUE NOT NULL,
              expression TEXT NOT NULL,
              notes TEXT,
              updated_at BIGINT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS messages (
              id TEXT PRIMARY KEY,
              role TEXT CHECK(role IN ('user','assistant')) NOT NULL,
              content TEXT NOT NULL,
              t BIGINT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS notion_sync_queue (
              message_id TEXT PRIMARY KEY,
              role TEXT CHECK(role IN ('user','assistant')) NOT NULL,
              content TEXT NOT NULL,
              t BIGINT NOT NULL,
              enqueued_at BIGINT NOT NULL,
              attempts INTEGER NOT NULL DEFAULT 0,
              last_attempt_at BIGINT,
              next_attempt_after BIGINT NOT NULL DEFAULT 0,
              last_error TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_notion_sync_next_attempt
              ON notion_sync_queue(next_attempt_after, enqueued_at);

            CREATE TABLE IF NOT EXISTS items (
              item TEXT PRIMARY KEY,
              type TEXT,
              role TEXT,
              launch_year INTEGER,
              base_price DOUBLE PRECISION,
              base_units DOUBLE PRECISION,
              growth DOUBLE PRECISION,
              margin DOUBLE PRECISION,
              adj_low DOUBLE PRECISION,
              adj_high DOUBLE PRECISION,
              adj_black_swan DOUBLE PRECISION,
              attributes JSONB NOT NULL DEFAULT '{}'::jsonb
            );

            CREATE TABLE IF NOT EXISTS item_field_definitions (
              field_key   TEXT PRIMARY KEY,
              label       TEXT NOT NULL,
              kind        TEXT NOT NULL,
              entity_kind TEXT NOT NULL DEFAULT 'offering',
              config      JSONB NOT NULL DEFAULT '{}'::jsonb,
              updated_at  BIGINT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS item_relationships (
              id SERIAL PRIMARY KEY,
              source_item TEXT NOT NULL REFERENCES items(item) ON DELETE CASCADE,
              target_item TEXT NOT NULL REFERENCES items(item) ON DELETE CASCADE,
              kind TEXT NOT NULL,
              weight DOUBLE PRECISION,
              metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
              created_at BIGINT NOT NULL,
              updated_at BIGINT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_item_relationships_kind ON item_relationships(kind);
            CREATE INDEX IF NOT EXISTS idx_item_relationships_source ON item_relationships(source_item);
            CREATE INDEX IF NOT EXISTS idx_item_relationships_target ON item_relationships(target_item);

            CREATE TABLE IF NOT EXISTS brand (
              field TEXT PRIMARY KEY,
              value TEXT
            );

            CREATE TABLE IF NOT EXISTS jobs (
              job_id TEXT PRIMARY KEY,
              type   TEXT NOT NULL CHECK(type IN ('base','econ','story','analysis')),
              status TEXT NOT NULL CHECK(status IN ('queued','running','succeeded','failed')),
              idempotency_key TEXT UNIQUE,
              created_at BIGINT NOT NULL,
              updated_at BIGINT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_jobs_status ON jobs(status);
            CREATE INDEX IF NOT EXISTS idx_jobs_updated_at ON jobs(updated_at);

            CREATE TABLE IF NOT EXISTS job_events (
              id SERIAL PRIMARY KEY,
              job_id TEXT NOT NULL REFERENCES jobs(job_id) ON DELETE CASCADE,
              type   TEXT NOT NULL,
              payload TEXT,
              t BIGINT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_job_events_job_id_t ON job_events(job_id, t);
            DO $$
            DECLARE
              r RECORD;
            BEGIN
              FOR r IN
                SELECT conname, oid
                FROM pg_constraint
                WHERE conrelid = 'jobs'::regclass AND contype = 'c'
              LOOP
                IF position('type' IN pg_get_constraintdef(r.oid)) > 0 THEN
                  IF position('analysis' IN pg_get_constraintdef(r.oid)) = 0 THEN
                    EXECUTE format('ALTER TABLE jobs DROP CONSTRAINT %I', r.conname);
                  END IF;
                END IF;
              END LOOP;
              IF NOT EXISTS (
                SELECT 1 FROM pg_constraint WHERE conrelid = 'jobs'::regclass AND conname = 'jobs_type_check'
              ) THEN
                EXECUTE 'ALTER TABLE jobs ADD CONSTRAINT jobs_type_check CHECK (type IN (''base'',''econ'',''story'',''analysis''))';
              END IF;
            END $$;

            CREATE TABLE IF NOT EXISTS job_cancellations (
              job_id TEXT PRIMARY KEY REFERENCES jobs(job_id) ON DELETE CASCADE,
              requested_at BIGINT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS model_versions (
              id SERIAL PRIMARY KEY,
              created_at BIGINT NOT NULL,
              label TEXT,
              payload TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_model_versions_created_at ON model_versions(created_at DESC);

            CREATE TABLE IF NOT EXISTS story_versions (
              id SERIAL PRIMARY KEY,
              created_at BIGINT NOT NULL,
              label TEXT,
              payload TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_story_versions_created_at ON story_versions(created_at DESC);

            CREATE TABLE IF NOT EXISTS project_meta (
              id INTEGER PRIMARY KEY CHECK(id=1),
              project_id TEXT NOT NULL,
              title TEXT NOT NULL,
              updated_at BIGINT NOT NULL,
              last_generated_offering TEXT
            );
            ALTER TABLE project_meta
              ADD COLUMN IF NOT EXISTS last_generated_offering TEXT;

            CREATE TABLE IF NOT EXISTS projects (
              id TEXT PRIMARY KEY,
              team_id TEXT,
              title TEXT NOT NULL,
              snapshot JSONB NOT NULL DEFAULT '{}'::jsonb,
              created_at BIGINT NOT NULL,
              updated_at BIGINT NOT NULL,
              archived_at BIGINT
            );
            ALTER TABLE projects ADD COLUMN IF NOT EXISTS owner_id TEXT REFERENCES users(id) ON DELETE SET NULL;
            ALTER TABLE projects ADD COLUMN IF NOT EXISTS workspace_id UUID REFERENCES workspaces(id);
            CREATE INDEX IF NOT EXISTS idx_projects_team ON projects(team_id);
            CREATE INDEX IF NOT EXISTS idx_projects_owner ON projects(owner_id);
            CREATE INDEX IF NOT EXISTS idx_projects_workspace_id ON projects(workspace_id);

            CREATE TABLE IF NOT EXISTS project_members (
              project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
              user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
              role TEXT NOT NULL CHECK(role IN ('owner','editor','viewer')) DEFAULT 'editor',
              added_at BIGINT NOT NULL,
              added_by TEXT REFERENCES users(id) ON DELETE SET NULL,
              PRIMARY KEY(project_id, user_id)
            );
            CREATE INDEX IF NOT EXISTS idx_project_members_user ON project_members(user_id);

            CREATE TABLE IF NOT EXISTS project_versions (
              id SERIAL PRIMARY KEY,
              project_id TEXT NOT NULL,
              created_at BIGINT NOT NULL,
              label TEXT,
              title TEXT,
              payload TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_project_versions_created_at ON project_versions(created_at DESC);

            CREATE TABLE IF NOT EXISTS sheet_plans (
              sheet TEXT PRIMARY KEY,
              steps_json TEXT NOT NULL,
              updated_at BIGINT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS story_slides (
              id SERIAL PRIMARY KEY,
              order_index INTEGER NOT NULL UNIQUE,
              title TEXT NOT NULL,
              bullets_json TEXT NOT NULL,
              note TEXT,
              style_json TEXT,
              images_json TEXT
            );

            CREATE TABLE IF NOT EXISTS story_slides_plan (
              id INTEGER PRIMARY KEY CHECK(id=1),
              steps_json TEXT NOT NULL,
              updated_at BIGINT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS pending_changes (
              id SERIAL PRIMARY KEY,
              scope TEXT NOT NULL,
              status TEXT NOT NULL CHECK(status IN ('pending','applied','discarded')) DEFAULT 'pending',
              title TEXT,
              summary TEXT,
              payload JSONB NOT NULL,
              documents JSONB NOT NULL DEFAULT '{}'::jsonb,
              meta JSONB NOT NULL DEFAULT '{}'::jsonb,
              created_at BIGINT NOT NULL,
              updated_at BIGINT NOT NULL,
              resolved_at BIGINT
            );
            CREATE INDEX IF NOT EXISTS idx_pending_changes_scope_status ON pending_changes(scope, status, created_at DESC);

            CREATE TABLE IF NOT EXISTS uploaded_files (
              id SERIAL PRIMARY KEY,
              project_id TEXT NOT NULL,
              artifact_id TEXT,
              kind TEXT NOT NULL CHECK(kind IN ('excel','powerpoint')),
              filename TEXT NOT NULL,
              size BIGINT NOT NULL,
              object_key TEXT NOT NULL,
              preview_json TEXT,
              analysis_json TEXT,
              uploaded_at BIGINT NOT NULL
            );
            ALTER TABLE uploaded_files
              ADD COLUMN IF NOT EXISTS artifact_id TEXT;

            CREATE INDEX IF NOT EXISTS idx_uploaded_files_project_kind ON uploaded_files(project_id, kind);
            CREATE INDEX IF NOT EXISTS idx_uploaded_files_artifact_id ON uploaded_files(artifact_id);

            CREATE TABLE IF NOT EXISTS analysis_documents (
              id SERIAL PRIMARY KEY,
              project_id TEXT NOT NULL,
              file_id INTEGER UNIQUE NOT NULL REFERENCES uploaded_files(id) ON DELETE CASCADE,
              source_kind TEXT NOT NULL CHECK(source_kind IN ('excel','powerpoint')),
              status TEXT NOT NULL CHECK(status IN ('pending','queued','processing','ready','failed')) DEFAULT 'pending',
              chunk_count INTEGER NOT NULL DEFAULT 0,
              embedding_dim INTEGER,
              checksum TEXT,
              meta JSONB DEFAULT '{}'::jsonb,
              last_error TEXT,
              last_job_id TEXT,
              created_at BIGINT NOT NULL,
              updated_at BIGINT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_analysis_documents_project ON analysis_documents(project_id);
            CREATE INDEX IF NOT EXISTS idx_analysis_documents_status ON analysis_documents(status);
            ALTER TABLE analysis_documents
              ADD COLUMN IF NOT EXISTS analysis_preview JSONB DEFAULT '{}'::jsonb;

            CREATE TABLE IF NOT EXISTS analysis_chunks (
              id SERIAL PRIMARY KEY,
              document_id INTEGER NOT NULL REFERENCES analysis_documents(id) ON DELETE CASCADE,
              chunk_index INTEGER NOT NULL,
              content TEXT NOT NULL,
              content_html TEXT,
              token_count INTEGER NOT NULL DEFAULT 0,
              embedding VECTOR(1536),
              meta JSONB NOT NULL DEFAULT '{}'::jsonb,
              created_at BIGINT NOT NULL,
              search_terms TSVECTOR GENERATED ALWAYS AS (to_tsvector('english', content)) STORED
            );
            CREATE INDEX IF NOT EXISTS idx_analysis_chunks_document_idx ON analysis_chunks(document_id, chunk_index);
            CREATE INDEX IF NOT EXISTS idx_analysis_chunks_meta_sheet ON analysis_chunks ((meta->>'sheet'));
            CREATE INDEX IF NOT EXISTS idx_analysis_chunks_meta_slide ON analysis_chunks ((meta->>'slide'));
            CREATE INDEX IF NOT EXISTS idx_analysis_chunks_search_terms ON analysis_chunks USING GIN (search_terms);
            CREATE INDEX IF NOT EXISTS idx_analysis_chunks_embedding ON analysis_chunks USING ivfflat (embedding vector_cosine_ops) WITH (lists = 100);

            CREATE TABLE IF NOT EXISTS telemetry_events (
              id SERIAL PRIMARY KEY,
              created_at BIGINT NOT NULL,
              category TEXT NOT NULL,
              service TEXT,
              stage TEXT,
              provider TEXT,
              model TEXT,
              request_id TEXT,
              document_id TEXT,
              latency_ms INTEGER,
              extra JSONB NOT NULL DEFAULT '{}'::jsonb
            );
            CREATE INDEX IF NOT EXISTS idx_telemetry_events_created_at ON telemetry_events(created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_telemetry_events_request ON telemetry_events(request_id);
            CREATE INDEX IF NOT EXISTS idx_telemetry_events_service ON telemetry_events(service, stage);

            CREATE TABLE IF NOT EXISTS user_sessions (
              id TEXT PRIMARY KEY,
              user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
              team_id TEXT REFERENCES teams(id) ON DELETE SET NULL,
              project_id TEXT REFERENCES projects(id) ON DELETE SET NULL,
              created_at BIGINT NOT NULL,
              last_seen_at BIGINT NOT NULL,
              presence_status TEXT,
              presence_message TEXT,
              user_agent TEXT,
              ip_address TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_user_sessions_user ON user_sessions(user_id);
            CREATE INDEX IF NOT EXISTS idx_user_sessions_team ON user_sessions(team_id);
            CREATE INDEX IF NOT EXISTS idx_user_sessions_project ON user_sessions(project_id);

            CREATE TABLE IF NOT EXISTS collaboration_events (
              id SERIAL PRIMARY KEY,
              team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
              user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
              event_type TEXT NOT NULL,
              payload JSONB NOT NULL DEFAULT '{}'::jsonb,
              created_at BIGINT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_collaboration_events_team ON collaboration_events(team_id, created_at DESC);
            
            -- Interactions table (unified chat/agent/system events)
            CREATE TABLE IF NOT EXISTS interactions (
              id SERIAL PRIMARY KEY,
              project_id TEXT NOT NULL,
              session_id TEXT NOT NULL,
              scope TEXT NOT NULL CHECK(scope IN ('user','agent','system')),
              type TEXT NOT NULL CHECK(type IN (
                'user_message','assistant_response','agent_request','agent_response','system_event'
              )),
              content TEXT NOT NULL,
              metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
              parent_id INTEGER REFERENCES interactions(id) ON DELETE CASCADE,
              created_at BIGINT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_interactions_project_scope 
              ON interactions(project_id, scope, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_interactions_session 
              ON interactions(session_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_interactions_type 
              ON interactions(type, created_at DESC);
            -- Ensure metadata column is JSONB (migration from TEXT if older deployments)
            DO $$
            BEGIN
              IF EXISTS (
                SELECT 1 FROM information_schema.columns
                WHERE table_name='interactions' AND column_name='metadata' AND data_type='text'
              ) THEN
                EXECUTE 'ALTER TABLE interactions ALTER COLUMN metadata TYPE JSONB USING metadata::jsonb';
              END IF;
            END $$;
            """
        )

        cx.execute(
            """
            CREATE TABLE IF NOT EXISTS ikam_fragments (
              id TEXT PRIMARY KEY,
              mime_type TEXT NOT NULL DEFAULT 'application/octet-stream',
              size BIGINT NOT NULL CHECK (size > 0),
              bytes BYTEA NOT NULL,
              created_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );
            CREATE INDEX IF NOT EXISTS idx_fragments_mime ON ikam_fragments(mime_type);

            CREATE TABLE IF NOT EXISTS ikam_artifacts (
              id UUID PRIMARY KEY,
              kind TEXT NOT NULL,
              title TEXT,
              project_id TEXT,
              status TEXT,
              root_fragment_id TEXT,
              created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
              updated_at TIMESTAMPTZ
            );
            CREATE INDEX IF NOT EXISTS idx_artifacts_kind ON ikam_artifacts(kind);
            CREATE INDEX IF NOT EXISTS idx_artifacts_created ON ikam_artifacts(created_at);
            CREATE INDEX IF NOT EXISTS idx_ikam_artifacts_project_id ON ikam_artifacts(project_id);
            CREATE INDEX IF NOT EXISTS idx_ikam_artifacts_status ON ikam_artifacts(status);
            CREATE INDEX IF NOT EXISTS idx_ikam_artifacts_updated_at ON ikam_artifacts(updated_at);

            ALTER TABLE ikam_artifacts
              ADD COLUMN IF NOT EXISTS root_fragment_id TEXT;
            ALTER TABLE ikam_artifacts
              ADD COLUMN IF NOT EXISTS head_object_id TEXT;

            CREATE TABLE IF NOT EXISTS ikam_artifact_workspaces (
              id TEXT PRIMARY KEY DEFAULT ('iaw-' || gen_random_uuid()::text),
              artifact_id UUID NOT NULL REFERENCES ikam_artifacts(id) ON DELETE CASCADE,
              base_ref JSONB NOT NULL,
              head_ref JSONB NOT NULL,
              status TEXT NOT NULL DEFAULT 'open',
              created_by TEXT,
              created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
              updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
              CONSTRAINT ikam_artifact_workspaces_status_check
                CHECK (status IN ('open', 'closed'))
            );
            CREATE INDEX IF NOT EXISTS idx_ikam_artifact_workspaces_artifact_id
              ON ikam_artifact_workspaces(artifact_id);
            CREATE INDEX IF NOT EXISTS idx_ikam_artifact_workspaces_status
              ON ikam_artifact_workspaces(status);
            CREATE INDEX IF NOT EXISTS idx_ikam_artifact_workspaces_updated_at
              ON ikam_artifact_workspaces(updated_at DESC);

            CREATE TABLE IF NOT EXISTS ikam_artifact_branches (
              id TEXT PRIMARY KEY DEFAULT ('iab-' || gen_random_uuid()::text),
              artifact_id UUID NOT NULL REFERENCES ikam_artifacts(id) ON DELETE CASCADE,
              name TEXT NOT NULL,
              base_commit_id TEXT,
              head_commit_id TEXT,
              status TEXT NOT NULL DEFAULT 'open',
              created_by TEXT,
              created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
              updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
              CONSTRAINT ikam_artifact_branches_status_check
                CHECK (status IN ('open', 'merged', 'abandoned'))
            );
            CREATE UNIQUE INDEX IF NOT EXISTS uniq_ikam_artifact_branches_artifact_name
              ON ikam_artifact_branches(artifact_id, name);
            CREATE INDEX IF NOT EXISTS idx_ikam_artifact_branches_artifact_id
              ON ikam_artifact_branches(artifact_id);
            CREATE INDEX IF NOT EXISTS idx_ikam_artifact_branches_status
              ON ikam_artifact_branches(status);

            CREATE TABLE IF NOT EXISTS ikam_artifact_commits (
              id TEXT PRIMARY KEY DEFAULT ('iac-' || gen_random_uuid()::text),
              artifact_id UUID NOT NULL REFERENCES ikam_artifacts(id) ON DELETE CASCADE,
              branch_id TEXT REFERENCES ikam_artifact_branches(id) ON DELETE SET NULL,
              parent_commit_id TEXT REFERENCES ikam_artifact_commits(id) ON DELETE SET NULL,
              base_ref JSONB NOT NULL,
              result_ref JSONB NOT NULL,
              staged_artifact_id UUID REFERENCES ikam_artifacts(id) ON DELETE SET NULL,
              delta_hash TEXT NOT NULL,
              view_hash TEXT NOT NULL,
              author TEXT,
              created_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );
            CREATE INDEX IF NOT EXISTS idx_ikam_artifact_commits_artifact_id
              ON ikam_artifact_commits(artifact_id);
            CREATE INDEX IF NOT EXISTS idx_ikam_artifact_commits_branch_id
              ON ikam_artifact_commits(branch_id);
            CREATE INDEX IF NOT EXISTS idx_ikam_artifact_commits_created_at
              ON ikam_artifact_commits(created_at DESC);
            CREATE UNIQUE INDEX IF NOT EXISTS uniq_ikam_artifact_commits_idempotency
              ON ikam_artifact_commits(
                artifact_id,
                delta_hash,
                view_hash,
                COALESCE(staged_artifact_id, '00000000-0000-0000-0000-000000000000'::uuid)
              );

            ALTER TABLE ikam_artifact_branches
              DROP CONSTRAINT IF EXISTS fk_ikam_artifact_branches_base_commit;

            DO $$
            BEGIN
              IF NOT EXISTS (
                SELECT 1 FROM pg_constraint WHERE conname = 'fk_ikam_artifact_branches_base_commit'
              ) THEN
                ALTER TABLE ikam_artifact_branches
                  ADD CONSTRAINT fk_ikam_artifact_branches_base_commit
                    FOREIGN KEY (base_commit_id)
                    REFERENCES ikam_artifact_commits(id)
                    ON DELETE SET NULL;
              END IF;
            END $$;

            ALTER TABLE ikam_artifact_branches
              DROP CONSTRAINT IF EXISTS fk_ikam_artifact_branches_head_commit;

            DO $$
            BEGIN
              IF NOT EXISTS (
                SELECT 1 FROM pg_constraint WHERE conname = 'fk_ikam_artifact_branches_head_commit'
              ) THEN
                ALTER TABLE ikam_artifact_branches
                  ADD CONSTRAINT fk_ikam_artifact_branches_head_commit
                    FOREIGN KEY (head_commit_id)
                    REFERENCES ikam_artifact_commits(id)
                    ON DELETE SET NULL;
              END IF;
            END $$;

            CREATE TABLE IF NOT EXISTS ikam_artifact_commit_parents (
              commit_id TEXT NOT NULL REFERENCES ikam_artifact_commits(id) ON DELETE CASCADE,
              parent_commit_id TEXT NOT NULL REFERENCES ikam_artifact_commits(id) ON DELETE CASCADE,
              parent_order INTEGER NOT NULL,
              PRIMARY KEY (commit_id, parent_order)
            );
            CREATE UNIQUE INDEX IF NOT EXISTS uniq_ikam_artifact_commit_parents_commit_parent
              ON ikam_artifact_commit_parents(commit_id, parent_commit_id);
            CREATE INDEX IF NOT EXISTS idx_ikam_artifact_commit_parents_parent
              ON ikam_artifact_commit_parents(parent_commit_id);

            CREATE TABLE IF NOT EXISTS ikam_artifact_versions (
              id TEXT PRIMARY KEY DEFAULT ('iav-' || gen_random_uuid()::text),
              artifact_id UUID NOT NULL REFERENCES ikam_artifacts(id) ON DELETE CASCADE,
              version_number INTEGER NOT NULL,
              data JSONB NOT NULL,
              content_hash TEXT,
              created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
              created_by TEXT,
              UNIQUE(artifact_id, version_number)
            );
            CREATE INDEX IF NOT EXISTS idx_ikam_artifact_versions_artifact_id
              ON ikam_artifact_versions(artifact_id);

            CREATE TABLE IF NOT EXISTS ikam_fragment_objects (
              object_id TEXT PRIMARY KEY,
              root_fragment_id TEXT NOT NULL REFERENCES ikam_fragments(id) ON DELETE RESTRICT,
              manifest JSONB NOT NULL,
              created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
              created_by UUID
            );
            CREATE INDEX IF NOT EXISTS idx_ikam_fragment_objects_object_id
              ON ikam_fragment_objects(object_id);
            CREATE INDEX IF NOT EXISTS idx_ikam_fragment_objects_root_fragment_id
              ON ikam_fragment_objects(root_fragment_id);

            DO $$
            BEGIN
              IF NOT EXISTS (
                SELECT 1 FROM pg_constraint WHERE conname = 'ikam_fragment_objects_root_fragment_id_fkey'
              ) THEN
                ALTER TABLE ikam_fragment_objects
                  ADD CONSTRAINT ikam_fragment_objects_root_fragment_id_fkey
                  FOREIGN KEY (root_fragment_id)
                  REFERENCES ikam_fragments(id)
                  ON DELETE RESTRICT;
              END IF;
            END $$;

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
            END $$;

            CREATE INDEX IF NOT EXISTS idx_ikam_artifacts_head_object_id
              ON ikam_artifacts(head_object_id);
            """
        )

        # Post-creation adjustments (idempotent)
        cx.execute("ALTER TABLE user_sessions ADD COLUMN IF NOT EXISTS project_id TEXT")
        cx.execute("CREATE INDEX IF NOT EXISTS idx_user_sessions_project ON user_sessions(project_id)")

        # Ensure IKAM extended schema
        from modelado.ikam_graph_schema import create_ikam_schema
        create_ikam_schema(cx._conn)
        _ensure_fragment_store_ref_scope(cx)

        # Seed model inputs if provided
        if seed_model_inputs:
            cx.executemany(
                """
                INSERT INTO model_inputs(key,value,notes,kind)
                VALUES (%s,%s,%s,%s)
                ON CONFLICT(key) DO UPDATE SET
                  value = EXCLUDED.value,
                  notes = EXCLUDED.notes,
                  kind = EXCLUDED.kind
                """,
                [
                    (
                        row["key"],
                        row.get("value", 0),
                        row.get("notes"),
                        row.get("kind", "number"),
                    )
                    for row in seed_model_inputs
                ],
            )


__all__ = [
    "ConnectionWrapper",
    "connection_scope",
    "ensure_schema",
    "get_connection",
    "reset_pool_for_pytest",
]
