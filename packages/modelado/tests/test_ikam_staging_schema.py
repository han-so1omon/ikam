import pytest
from modelado.db import connection_scope
from modelado.ikam_graph_schema import create_ikam_schema

def test_staging_table_creation():
    with connection_scope() as cx:
        create_ikam_schema(cx)
        with cx.cursor() as cur:
            # Check if table exists
            cur.execute("SELECT to_regclass('public.ikam_staging_fragments')")
            res = cur.fetchone()
            # Handle both tuple and dict row factories
            val = res[0] if isinstance(res, tuple) else res["to_regclass"]
            assert val is not None
            
            # Check columns
            cur.execute("""
                SELECT column_name, data_type 
                FROM information_schema.columns 
                WHERE table_name = 'ikam_staging_fragments'
            """)
            rows = cur.fetchall()
            cols = {}
            for row in rows:
                if isinstance(row, tuple):
                    cols[row[0]] = row[1]
                else:
                    cols[row["column_name"]] = row["data_type"]
            
            assert "session_id" in cols
            assert "id" in cols
            assert "bytes" in cols
