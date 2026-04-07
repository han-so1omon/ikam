import json
import os
import uuid
from pathlib import Path
from typing import Dict, Any

from modelado.db import connection_scope
from ikam.graph import _cas_hex

from modelado.hugegraph_client import HugeGraphClient
from modelado.hugegraph_projection import ensure_ikam_projection_schema, _ensure_vertex

def resolve_local_references(data: Any, id_map: Dict[str, str]) -> Any:
    if isinstance(data, dict):
        new_data = {}
        for k, v in data.items():
            if k == "id" and isinstance(v, str) and v in id_map:
                new_data[k] = id_map[v]
            elif isinstance(v, (dict, list)):
                new_data[k] = resolve_local_references(v, id_map)
            elif isinstance(v, str) and v in id_map:
                new_data[k] = id_map[v]
            else:
                new_data[k] = v
        return new_data
    elif isinstance(data, list):
        return [resolve_local_references(item, id_map) for item in data]
    return data

def load_graph_fixture(fixture_path: Path):
    with open(fixture_path, 'r', encoding='utf-8') as f:
        fixture = json.load(f)
        
    graph_id = fixture.get("graph_id", "default_graph")
    fragments = fixture.get("fragments", [])
    
    # 1. First pass to generate UUIDs for local fragment_ids
    id_map: Dict[str, str] = {}
    for frag in fragments:
        local_id = frag.get("fragment_id")
        if local_id and not local_id.startswith("uuid-"):
            id_map[local_id] = str(uuid.uuid4())

    base_url = (os.getenv("HUGEGRAPH_URL") or os.getenv("HUGEGRAPH_BASE_URL") or "http://localhost:8080").strip()
    graph = (os.getenv("HUGEGRAPH_GRAPH") or "hugegraph").strip()
    
    hg_client = HugeGraphClient(base_url=base_url, graph=graph)
    ensure_ikam_projection_schema(hg_client)
            
    with connection_scope() as cx:
        with cx.cursor() as cur:
            for frag in fragments:
                local_id = frag.get("fragment_id")
                actual_id = id_map.get(local_id, local_id) if local_id else str(uuid.uuid4())
                
                # Resolve references in the value
                value = resolve_local_references(frag.get("value", {}), id_map)
                
                # Compute CAS ID over the resolved value and mime_type
                content_for_hash = json.dumps({"value": value, "mime_type": frag.get("mime_type", "")}, sort_keys=True)
                cas_id = _cas_hex(content_for_hash.encode('utf-8'))
                mime_type = frag.get("mime_type", "")
                
                cur.execute(
                    """
                    INSERT INTO ikam_fragment_store (cas_id, env, project_id, mime_type, value)
                    VALUES (%s, 'dev', %s, %s, %s)
                    ON CONFLICT (cas_id, env, COALESCE(operation_id, '')) DO NOTHING
                    """,
                    (cas_id, graph_id, mime_type, json.dumps(value))
                )

                # Seed to hugegraph to ensure the graph is well-formed
                props = {
                    "type": value.get("type", "fragment"),
                    "mime_type": mime_type,
                }
                _ensure_vertex(
                    client=hg_client, 
                    id=cas_id, 
                    project_id=graph_id, 
                    properties=props
                )
                        
        cx.commit()

def load_all_fixtures(fixtures_dir: Path):
    for path in fixtures_dir.glob("*.json"):
        load_graph_fixture(path)

if __name__ == "__main__":
    base_dir = Path(__file__).resolve().parent.parent.parent.parent.parent / "modelado" / "fixtures" / "graphs"
    print(f"Loading from {base_dir}")
    load_all_fixtures(base_dir)
