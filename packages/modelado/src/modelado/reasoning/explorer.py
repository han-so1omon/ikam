from __future__ import annotations
from typing import Optional
from psycopg import Connection
from modelado.reasoning.query import SemanticQuery, Subgraph
from modelado.reasoning.registry import get_search_strategy_registry

class GraphExplorer:
    """Service to coordinate constrained graph traversal and discovery."""
    
    @staticmethod
    def discover(cx: Connection, query: SemanticQuery) -> Subgraph:
        """
        Discover a subgraph based on the provided query constraints.
        
        This is the primary entry point for the 'Discovery' phase of the SQI Framework.
        It respects D19 isolation by delegating to environment-aware search strategies.
        """
        strategy_name = query.search_strategy.name or "default"
        registry = get_search_strategy_registry(cx)
        strategy_fn = registry.get(strategy_name)

        if not strategy_fn:
            raise ValueError(f"No search strategy registered for name: {strategy_name}")
            
        # Execute the discovery strategy
        # Strategies are expected to return a Subgraph model.
        subgraph = strategy_fn(cx, query.constraints)
        
        return subgraph
