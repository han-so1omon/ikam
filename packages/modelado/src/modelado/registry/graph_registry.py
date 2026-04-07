from typing import Optional, Dict, Any
from modelado.db import connection_scope

class GraphRegistry:
    """
    Looks up elements in a graph-backed registry fragment.
    Registries are StructuredDataIRs with profile='registry' and a specific registry_type.
    """
    def __init__(self, registry_type: str):
        self.registry_type = registry_type

    def get_fragment_id(self, logical_name: str) -> Optional[str]:
        with connection_scope() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    """
                    SELECT value->'entries'->>%s as frag_id
                    FROM ikam_fragment_store 
                    WHERE value->>'profile' = 'registry'
                      AND value->>'registry_type' = %s
                    ORDER BY created_at DESC LIMIT 1
                    """,
                    (logical_name, self.registry_type)
                )
                row = cur.fetchone()
                if row and row["frag_id"]:
                    return row["frag_id"]
        return None

    def get_fragment_value(self, logical_name: str) -> Optional[Dict[str, Any]]:
        frag_id = self.get_fragment_id(logical_name)
        if not frag_id:
            return None
            
        with connection_scope() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    "SELECT value FROM ikam_fragment_store WHERE fragment_id = %s",
                    (frag_id,)
                )
                row = cur.fetchone()
                if row and row["value"]:
                    return row["value"]
        return None
