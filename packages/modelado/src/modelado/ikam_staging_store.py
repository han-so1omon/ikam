from typing import Any, List
from ikam.fragments import Fragment
from ikam.adapters import v3_fragment_to_cas_bytes

try:
    from modelado import ikam_metrics
except ImportError:  # pragma: no cover - metrics optional in some environments
    ikam_metrics = None

class StagingStore:
    def __init__(self, cx: Any):
        self._cx = cx

    def stage_fragment(self, fragment: Fragment, session_id: str) -> None:
        if not fragment.cas_id:
            # ensure cas_id is computed
            cas_bytes = v3_fragment_to_cas_bytes(fragment)
            from ikam.graph import _cas_hex
            fragment.cas_id = _cas_hex(cas_bytes)
        
        # Get raw bytes for storage
        raw_bytes = v3_fragment_to_cas_bytes(fragment)
        
        self._cx.execute(
            """
            INSERT INTO ikam_staging_fragments (id, session_id, mime_type, size, bytes)
            VALUES (%s, %s, %s, %s, %s)
            ON CONFLICT (session_id, id) DO NOTHING
            """,
            (fragment.cas_id, session_id, fragment.mime_type, len(raw_bytes), raw_bytes)
        )

    def promote_session(self, session_id: str) -> int:
        """Promote all fragments in session to permanent storage."""
        def _count(row: Any) -> int:
            if isinstance(row, tuple):
                return int(row[0])
            return int(next(iter(row.values())))

        with self._cx.cursor() as cur:
            cur.execute(
                """
                SELECT COUNT(*)
                FROM ikam_staging_fragments
                WHERE session_id = %s
                """,
                (session_id,),
            )
            total_staged = _count(cur.fetchone())

            cur.execute(
                """
                SELECT COUNT(*)
                FROM ikam_staging_fragments s
                JOIN ikam_fragments f ON f.id = s.id
                WHERE s.session_id = %s
                """,
                (session_id,),
            )
            existing = _count(cur.fetchone())

        misses = max(0, total_staged - existing)
        hits = max(0, existing)

        # Simple bulk copy with ON CONFLICT DO NOTHING
        with self._cx.cursor() as cur:
            cur.execute(
                """
                INSERT INTO ikam_fragments (id, mime_type, size, bytes)
                SELECT id, mime_type, size, bytes
                FROM ikam_staging_fragments
                WHERE session_id = %s
                ON CONFLICT (id) DO NOTHING
                """,
                (session_id,)
            )
            promoted = cur.rowcount

        if ikam_metrics:
            for _ in range(misses):
                ikam_metrics.record_cas_miss()
            for _ in range(hits):
                ikam_metrics.record_cas_hit()

        return promoted
