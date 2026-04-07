from __future__ import annotations
from typing import Any, List, Set, Dict, Union
from modelado.reasoning.query import QueryConstraints, Subgraph
from modelado.reasoning.registry import get_search_strategy_registry
from modelado.db import connection_scope
from modelado.core.execution_context import execution_context, ExecutionContext, ExecutionMode, WriteScope
from ikam_perf_report.benchmarks.store import STORE
from ikam.ir.core import PropositionIR, ExpressionIR, StructuredDataIR
from modelado.graph_edge_event_log import GraphEdgeEvent

def _hydrate_ir_from_store(fragment: Any) -> Union[PropositionIR, ExpressionIR, StructuredDataIR]:
    """Hydrate a fragment payload from BenchmarkStore into an IR model."""
    if not fragment:
        raise ValueError("Hydration failed: missing fragment payload")
        
    mime = str(fragment.get("mime_type") or "")
    value = fragment.get("value")
    fragment_id = str(fragment.get("id") or "")
    
    if not value or not fragment_id:
        raise ValueError("Hydration failed: fragment missing id or value")
        
    try:
        if "application/ikam-proposition" in mime:
            ir_node = PropositionIR.model_validate(value)
        elif "application/ikam-expression" in mime:
            ir_node = ExpressionIR.model_validate(value)
        elif "application/ikam-structured-data" in mime:
            ir_node = StructuredDataIR.model_validate(value)
        else:
            raise ValueError(f"Hydration failed: unsupported mime_type '{mime}'")
    except ValueError as exc:
        error_text = str(exc)
        if error_text.startswith("Hydration failed:"):
            raise
        raise ValueError(f"Hydration failed: invalid payload for mime_type '{mime}'")
    except Exception:
        raise ValueError(f"Hydration failed: invalid payload for mime_type '{mime}'")

    ir_node.fragment_id = fragment_id
    return ir_node

def benchmark_bfs_search(cx: Any, constraints: QueryConstraints) -> Subgraph:
    """BFS traversal for BenchmarkStore (ignores cx)."""
    anchor_ids: List[str] = constraints.extra_filters.get("anchor_ids", [])
    if not anchor_ids:
        return Subgraph()
        
    graph_id = constraints.extra_filters.get("graph_id")
    graph = STORE.get_graph(graph_id) if graph_id else STORE.latest_graph()
    if not graph:
        return Subgraph()
        
    env = constraints.env_scope
    
    # Build a lookup for edges in the graph snapshot
    # BenchmarkStore GraphSnapshot has 'edges' as List[Dict[str, Any]]
    
    discovered_nodes: Set[str] = set(anchor_ids)
    discovered_edges: List[GraphEdgeEvent] = []
    
    queue = [(fid, 0) for fid in anchor_ids]
    visited = set(anchor_ids)
    
    while queue:
        current_id, depth = queue.pop(0)
        if depth >= constraints.max_hops:
            continue
            
        # Find all outgoing edges from this node in the snapshot
        for edge in graph.edges:
            source = str(edge.get("source"))
            if source != current_id:
                continue
                
            # D19 Isolation Check
            if env:
                # In BenchmarkStore, env info is usually in fragment meta or edge meta
                edge_meta = edge.get("meta") or {}
                event_env_type = edge_meta.get("env_type") or edge_meta.get("envType")
                event_env_id = edge_meta.get("env_id") or edge_meta.get("envId")
                
                if event_env_type != env.env_type or event_env_id != env.env_id:
                    # Allow committed from dev
                    if env.env_type == "dev" and event_env_type == "committed":
                        pass
                    else:
                        raise ValueError(
                            f"Unauthorized scope traversal from {env.env_type}:{env.env_id} "
                            f"to {event_env_type}:{event_env_id}"
                        )
            
            target = str(edge.get("target"))
            kind = str(edge.get("kind") or "composition")
            
            # Map Snapshot edge to GraphEdgeEvent for Subgraph compatibility
            event = GraphEdgeEvent(
                id=hash(f"{source}-{target}-{kind}"),
                project_id=graph.graph_id,
                op="upsert",
                edge_label=f"knowledge:{kind}",
                out_id=source,
                in_id=target,
                properties=edge.get("meta") or {},
                t=0,
                idempotency_key=None
            )
            
            discovered_edges.append(event)
            if target not in visited:
                visited.add(target)
                discovered_nodes.add(target)
                queue.append((target, depth + 1))
                
    # Hydrate nodes from graph.fragments
    # First, find fragments by ID
    fragment_lookup = {str(STORE._fragment_to_payload(f).get("id")): STORE._fragment_to_payload(f) for f in graph.fragments}
    
    hydrated_nodes = []
    for node_id in discovered_nodes:
        payload = fragment_lookup.get(node_id)
        ir_node = _hydrate_ir_from_store(payload)
        # Filter by node_types if specified
        if constraints.node_types:
            kind = ir_node.__class__.__name__.replace("IR", "")
            if kind not in constraints.node_types:
                continue
        hydrated_nodes.append(ir_node)
            
    return Subgraph(
        nodes=hydrated_nodes,
        edges=discovered_edges,
        anchor_ids=anchor_ids
    )

def register_benchmark_strategies():
    ctx = ExecutionContext(
        mode=ExecutionMode.BACKGROUND,
        actor_id="system",
        purpose="register_benchmark_strategies",
        write_scope=WriteScope(allowed=True, project_id="global_registry", operation="registry_update")
    )
    with execution_context(ctx), connection_scope() as cx:
        reg = get_search_strategy_registry(cx)
        reg.register("benchmark_bfs", benchmark_bfs_search)
