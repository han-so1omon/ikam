import pytest
import base64
from ikam.fragments import Fragment
from modelado.db import connection_scope
from modelado.ikam_graph_schema import create_ikam_schema, truncate_ikam_tables
from modelado.ikam_staging_store import StagingStore
from modelado import ikam_metrics

@pytest.fixture
def store():
    with connection_scope() as cx:
        create_ikam_schema(cx)
        truncate_ikam_tables(cx)
        # truncate staging too
        with cx.cursor() as cur:
            cur.execute("TRUNCATE ikam_staging_fragments")
        yield StagingStore(cx)

def test_stage_fragment(store):
    frag = Fragment(value={"hello": "world"}, mime_type="application/json")
    # Helper to compute cas_id if needed, or StagingStore does it? 
    # Let's assume StagingStore handles it or we use ikam helpers
    from ikam.adapters import v3_fragment_to_cas_bytes
    from ikam.graph import _cas_hex
    cas_bytes = v3_fragment_to_cas_bytes(frag)
    frag.cas_id = _cas_hex(cas_bytes)
    
    session_id = "sess-001"
    store.stage_fragment(frag, session_id)
    
    # Verify in DB
    with store._cx.cursor() as cur:
        cur.execute("SELECT id FROM ikam_staging_fragments WHERE session_id = %s", (session_id,))
        row = cur.fetchone()
        assert row is not None
        # Handle dict or tuple row
        val = row[0] if isinstance(row, tuple) else row["id"]
        assert val == frag.cas_id

def test_promote_session(store):
    # Create a fragment in staging
    frag = Fragment(value="persistent", mime_type="text/plain")
    from ikam.adapters import v3_fragment_to_cas_bytes
    from ikam.graph import _cas_hex
    cas_bytes = v3_fragment_to_cas_bytes(frag)
    frag.cas_id = _cas_hex(cas_bytes)
    
    session_id = "sess-002"
    store.stage_fragment(frag, session_id)
    
    # Promote
    promoted_count = store.promote_session(session_id)
    assert promoted_count == 1
    
    # Verify in permanent table
    with store._cx.cursor() as cur:
        cur.execute("SELECT id FROM ikam_fragments WHERE id = %s", (frag.cas_id,))
        assert cur.fetchone() is not None


def test_promote_session_records_cas_hits_and_misses(store):
    frag = Fragment(value={"poc": "metrics"}, mime_type="application/json")
    from ikam.adapters import v3_fragment_to_cas_bytes
    from ikam.graph import _cas_hex

    cas_bytes = v3_fragment_to_cas_bytes(frag)
    frag.cas_id = _cas_hex(cas_bytes)

    session_id = "sess-metrics"
    store.stage_fragment(frag, session_id)

    before_hits = int(ikam_metrics.cas_hits_total._value.get())
    before_misses = int(ikam_metrics.cas_misses_total._value.get())

    first_promoted = store.promote_session(session_id)
    second_promoted = store.promote_session(session_id)

    after_hits = int(ikam_metrics.cas_hits_total._value.get())
    after_misses = int(ikam_metrics.cas_misses_total._value.get())

    assert first_promoted == 1
    assert second_promoted == 0
    assert after_misses - before_misses >= 1
    assert after_hits - before_hits >= 1
