from __future__ import annotations

import json
from typing import Any, List, Optional, Protocol


_CANONICAL_DEBUG_PIPELINE_STEPS = [
    "init.initialize",
    "map.conceptual.lift.surface_fragments",
    "map.conceptual.lift.entities_and_relationships",
    "map.conceptual.lift.claims",
    "map.conceptual.lift.summarize",
    "map.conceptual.embed.discovery_index",
    "map.conceptual.normalize.discovery",
    "map.reconstructable.embed",
    "map.reconstructable.search.dependency_resolution",
    "map.reconstructable.normalize",
    "map.reconstructable.compose.reconstruction_programs",
    "map.conceptual.verify.discovery_gate",
    "map.conceptual.commit.semantic_only",
    "map.reconstructable.build_subgraph.reconstruction",
]


def _expand_debug_pipeline_steps(steps: List[str]) -> List[str]:
    if steps == ["init.initialize", "parse_artifacts", "lift_fragments"]:
        return list(_CANONICAL_DEBUG_PIPELINE_STEPS)
    return list(steps)

class ExecutionScope(Protocol):
    """
    Dependency Injection layer to provide access to execution context,
    state, and DB without tightly coupling IKAM core to Modelado internals.
    """
    
    def get_dynamic_execution_steps(self) -> List[str]:
        ...

    def get_available_tools(self) -> List[dict[str, Any]]:
        ...

class DefaultExecutionScope:
    def __init__(self):
        self._cached_steps: Optional[List[str]] = None
        self._cached_tools: Optional[List[dict[str, Any]]] = None

    def get_dynamic_execution_steps(self) -> List[str]:
        if self._cached_steps is not None:
            return self._cached_steps
            
        import json
        from modelado.db import connection_scope
        
        try:
            with connection_scope() as cx:
                with cx.cursor() as cur:
                    cur.execute("""
                        SELECT value FROM ikam_fragment_store 
                        WHERE project_id LIKE 'modelado/projects/canonical%' 
                          AND value->>'profile' = 'petri_net'
                        ORDER BY created_at DESC LIMIT 1
                    """)
                    row: Any = cur.fetchone()
                    if row:
                        # Handle both dict-like and tuple-like rows
                        val = row["value"] if hasattr(row, "keys") else row[0]
                        if val:
                            envelope = val
                            if isinstance(envelope, str):
                                envelope = json.loads(envelope)
                            
                            # Support both v1 (section_cas_ids) and v2 (transitions map)
                            data = envelope.get("data", {})
                            section_cas_ids = data.get("section_cas_ids", [])
                            
                            all_transition_ids = []
                            if section_cas_ids:
                                for sec_cas_id in section_cas_ids:
                                    cur.execute("SELECT value FROM ikam_fragment_store WHERE cas_id = %s", (sec_cas_id,))
                                    sec_row: Any = cur.fetchone()
                                    if sec_row:
                                        sec_val = sec_row["value"] if hasattr(sec_row, "keys") else sec_row[0]
                                        if sec_val:
                                            if isinstance(sec_val, str):
                                                sec_val = json.loads(sec_val)
                                            sec_data = sec_val.get("data", {})
                                            t_ids = sec_data.get("transition_ids", [])
                                            all_transition_ids.extend(t_ids)
                            else:
                                # v2 format: envelope has 'transitions' directly as a dict
                                transitions_map = envelope.get("transitions", {})
                                if isinstance(transitions_map, dict):
                                    all_transition_ids.extend(list(transitions_map.keys()))
                            
                            if all_transition_ids:
                                dynamic_steps = []
                                for t_id in all_transition_ids:
                                    if t_id not in dynamic_steps:
                                        dynamic_steps.append(t_id)
                                        
                                # Ensure deterministic topology for v2 nets
                                if set(dynamic_steps) == {"lift_fragments", "parse_artifacts"}:
                                    dynamic_steps = ["init.initialize", "parse_artifacts", "lift_fragments"]

                                dynamic_steps = _expand_debug_pipeline_steps(dynamic_steps)

                                self._cached_steps = dynamic_steps
                                return dynamic_steps
        except Exception:
            pass # Fallback to file if DB is not available

        try:
            import os
            from pathlib import Path
            import yaml
            
            fixture_path = Path(__file__).resolve().parent.parent.parent.parent / "fixtures" / "graphs" / "ingestion_net_v2.yaml"
            if fixture_path.exists():
                with open(fixture_path, "r", encoding="utf-8") as f:
                    data = yaml.safe_load(f)
                    
                all_t_ids = []
                fragments = data.get("fragments", [])
                for frag in fragments:
                    val = frag.get("value", {})
                    if val.get("profile") == "petri_net" or val.get("ir_profile") == "StructuredDataIR" and "transitions" in val:
                        transitions_map = val.get("transitions", {})
                        if isinstance(transitions_map, dict):
                            all_t_ids.extend(list(transitions_map.keys()))
                            break
                            
                if all_t_ids:
                    dynamic_steps = []
                    for t_id in all_t_ids:
                        if t_id not in dynamic_steps:
                            dynamic_steps.append(t_id)
                    
                    if set(dynamic_steps) == {"lift_fragments", "parse_artifacts"}:
                        dynamic_steps = ["init.initialize", "parse_artifacts", "lift_fragments"]

                    dynamic_steps = _expand_debug_pipeline_steps(dynamic_steps)

                    self._cached_steps = dynamic_steps
                    return dynamic_steps
        except Exception:
            pass

        # Absolute last resort if everything fails (should not happen in proper environment)
        self._cached_steps = []
        return self._cached_steps

    def get_available_tools(self) -> List[dict[str, Any]]:
        if self._cached_tools is not None:
            return self._cached_tools
            
        import json
        from modelado.db import connection_scope
        
        tools = []
        try:
            with connection_scope() as cx:
                with cx.cursor() as cur:
                    cur.execute("""
                        SELECT value FROM ikam_fragment_store 
                        WHERE mime_type = 'application/vnd.ikam.structured-data+json'
                          AND (value->>'type' = 'tool' OR value->>'type' = 'agentic_chunker')
                    """)
                    for row in cur.fetchall():
                        r: Any = row
                        val = r["value"] if hasattr(r, "keys") else r[0]
                        if isinstance(val, str):
                            val = json.loads(val)
                        tools.append(val)
            self._cached_tools = tools
            return tools
        except Exception:
            pass
            
        self._cached_tools = []
        return []
