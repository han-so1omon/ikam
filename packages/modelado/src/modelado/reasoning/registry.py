from __future__ import annotations
from typing import Any, Callable
import psycopg
from modelado.registry import ReasoningRegistryAdapter, get_shared_registry_manager

def get_search_strategy_registry(cx: psycopg.Connection[Any]) -> ReasoningRegistryAdapter[Callable[..., Any]]:
    return ReasoningRegistryAdapter(
        cx,
        get_shared_registry_manager(),
        "reasoning.search_strategies",
    )

def get_interpretation_registry(cx: psycopg.Connection[Any]) -> ReasoningRegistryAdapter[Callable[..., Any]]:
    return ReasoningRegistryAdapter(
        cx,
        get_shared_registry_manager(),
        "reasoning.interpretation_handlers",
    )

def bootstrap_reasoning_registry(cx: psycopg.Connection[Any]) -> None:
    from modelado.reasoning.strategies import bfs_search, dfs_search
    reg = get_search_strategy_registry(cx)
    existing_keys = set(reg.list_keys())
    if "bfs" not in existing_keys:
        reg.register("bfs", bfs_search)
    if "dfs" not in existing_keys:
        reg.register("dfs", dfs_search)
    if "default" not in existing_keys:
        reg.register("default", bfs_search)
