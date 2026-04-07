from __future__ import annotations
from typing import List, Set, Union, Optional
from psycopg import Connection

from ikam.ir.core import PropositionIR, ExpressionIR, StructuredDataIR
from modelado.environment_scope import scope_ref_from_qualifiers
from modelado.reasoning.query import QueryConstraints, Subgraph
from modelado.graph_edge_event_log import list_graph_edge_events, GraphEdgeEvent


def _is_visible_ref(event_ref: str | None, constraints: QueryConstraints) -> bool:
    if event_ref is None:
        return False
    env = constraints.env_scope
    if env is None:
        return True
    if event_ref == env.ref:
        return True
    return event_ref in constraints.base_refs


def _require_visible_event_scope(event: GraphEdgeEvent, constraints: QueryConstraints) -> None:
    env = constraints.env_scope
    if env is None:
        return

    event_ref = scope_ref_from_qualifiers(event.properties)
    if _is_visible_ref(event_ref, constraints):
        return

    if event_ref is not None:
        raise ValueError(f"Unauthorized scope traversal from ref {env.ref} to {event_ref}")

    raise ValueError("Graph edge event missing canonical ref qualifier")

def _hydrate_ir(fragment_id: str, cx: Connection) -> Optional[Union[PropositionIR, ExpressionIR, StructuredDataIR]]:
    """Hydrate a fragment ID into its respective IR model by MIME type."""
    from ikam.adapters import v3_fragment_from_cas_bytes
    from modelado.ikam_graph_repository import get_fragment_by_id
    
    storage_frag = get_fragment_by_id(cx, fragment_id)
    if not storage_frag:
        return None
    
    # We use v3_fragment_from_cas_bytes to get the value/mime_type
    # The storage_frag contains raw bytes
    v3_frag = v3_fragment_from_cas_bytes(cas_id=fragment_id, payload=storage_frag.bytes)
    
    mime = v3_frag.mime_type
    if not mime:
        return None
        
    ir_node = None
    if "application/ikam-proposition" in mime:
        ir_node = PropositionIR.model_validate(v3_frag.value)
    elif "application/ikam-expression" in mime:
        ir_node = ExpressionIR.model_validate(v3_frag.value)
    elif "application/ikam-structured-data" in mime:
        ir_node = StructuredDataIR.model_validate(v3_frag.value)
    
    if ir_node:
        # Set the fragment_id on the IR node for attribution/D18
        ir_node.fragment_id = fragment_id
        
    return ir_node

def bfs_search(cx: Connection, constraints: QueryConstraints) -> Subgraph:
    """Breadth-first discovery of the subgraph, honoring D19 isolation."""
    anchor_ids: List[str] = constraints.extra_filters.get("anchor_ids", [])
    if not anchor_ids:
        return Subgraph()
        
    project_id = constraints.extra_filters.get("project_id", "default-project")
    env = constraints.env_scope
    
    discovered_nodes: Set[str] = set(anchor_ids)
    discovered_edges: List[GraphEdgeEvent] = []
    
    queue = [(fid, 0) for fid in anchor_ids]
    visited = set(anchor_ids)
    
    while queue:
        current_id, depth = queue.pop(0)
        if depth >= constraints.max_hops:
            continue
            
        # Find all outgoing edges from this node
        # In a real system, we'd have a graph index, 
        # but here we'll use list_graph_edge_events as an approximation
        # for a small project or we'd need a more efficient query.
        
        # Optimization: list_graph_edge_events(out_id=current_id)
        events = list_graph_edge_events(cx, project_id=project_id, out_id=current_id)
        
        for event in events:
            # D19 Isolation Check
            if env:
                _require_visible_event_scope(event, constraints)
            
            # Filter by edge label if needed (e.g. only knowledge:*)
            if not event.edge_label.startswith("knowledge:"):
                continue
                
            discovered_edges.append(event)
            if event.in_id not in visited:
                visited.add(event.in_id)
                discovered_nodes.add(event.in_id)
                queue.append((event.in_id, depth + 1))
                
    # Hydrate nodes
    hydrated_nodes = []
    for node_id in discovered_nodes:
        ir_node = _hydrate_ir(node_id, cx)
        if ir_node:
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

def dfs_search(cx: Connection, constraints: QueryConstraints) -> Subgraph:
    """Depth-first discovery of the subgraph. For now, a simplified version of BFS."""
    # TODO: Implement actual DFS traversal logic if it differs significantly for Pollock chains
    return bfs_search(cx, constraints)
