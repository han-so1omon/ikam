"""Performance benchmark for execution link traversal (CTE vs N+1).

This test is skipped unless TEST_DATABASE_URL is set and BENCH_EXEC_GRAPH=1.
It measures recursive CTE subtree fetch vs N+1 child fetches on the same dataset.
"""

from __future__ import annotations

import os
import time
from typing import List, Tuple

import pytest
import pytest_asyncio

try:  # psycopg3 async pool
    from psycopg_pool import AsyncConnectionPool
except Exception:  # pragma: no cover - optional dependency
    AsyncConnectionPool = None  # type: ignore

from modelado.core.execution_links_async import (
    AsyncExecutionLinkGraph,
    create_execution_links_schema_async,
)


DATASET = {
    "children": 10,
    "grandchildren_per_child": 10,
}


@pytest_asyncio.fixture
async def maybe_pool():
    db_url = os.getenv("TEST_DATABASE_URL")
    if not db_url or not os.getenv("BENCH_EXEC_GRAPH"):
        pytest.skip("Benchmark requires TEST_DATABASE_URL and BENCH_EXEC_GRAPH=1")
    if AsyncConnectionPool is None:
        pytest.skip("psycopg_pool not available")
    pool = AsyncConnectionPool(conninfo=db_url, open=False)
    await pool.open()
    try:
        # Ensure schema exists and clean
        async with pool.connection() as cx:
            await create_execution_links_schema_async(cx)
            async with cx.cursor() as cur:
                await cur.execute("TRUNCATE execution_links")
            await cx.commit()
        yield pool
    finally:
        await pool.close()


async def _seed_links(pool: AsyncConnectionPool) -> Tuple[str, int]:
    root = "exec_root"
    total_links = 0
    async with pool.connection() as cx:
        async with cx.cursor() as cur:
            # Insert children
            for i in range(DATASET["children"]):
                child = f"exec_child_{i}"
                await cur.execute(
                    """
                    INSERT INTO execution_links (
                        link_id, caller_execution_id, callee_execution_id,
                        caller_function_id, callee_function_id, invocation_order,
                        context_snapshot, materialized_path, depth
                    ) VALUES (%s, %s, %s, %s, %s, %s, '{}', %s, %s)
                    """,
                    (
                        f"link_child_{i}",
                        root,
                        child,
                        "f_root",
                        "f_child",
                        i,
                        f"{root}/{child}",
                        1,
                    ),
                )
                total_links += 1
                # Insert grandchildren
                for j in range(DATASET["grandchildren_per_child"]):
                    grand = f"exec_grand_{i}_{j}"
                    await cur.execute(
                        """
                        INSERT INTO execution_links (
                            link_id, caller_execution_id, callee_execution_id,
                            caller_function_id, callee_function_id, invocation_order,
                            context_snapshot, materialized_path, depth
                        ) VALUES (%s, %s, %s, %s, %s, %s, '{}', %s, %s)
                        """,
                        (
                            f"link_grand_{i}_{j}",
                            child,
                            grand,
                            "f_child",
                            "f_grand",
                            j,
                            f"{root}/{child}/{grand}",
                            2,
                        ),
                    )
                    total_links += 1
        await cx.commit()
    return root, total_links


async def _n_plus_one_traversal(pool: AsyncConnectionPool, root: str) -> List[str]:
    seen: List[str] = []
    queue = [root]
    while queue:
        node = queue.pop(0)
        seen.append(node)
        async with pool.connection() as cx:
            async with cx.cursor() as cur:
                await cur.execute(
                    """
                    SELECT callee_execution_id
                    FROM execution_links
                    WHERE caller_execution_id = %s
                    ORDER BY invocation_order ASC
                    """,
                    (node,),
                )
                rows = await cur.fetchall()
                queue.extend([r[0] for r in rows])
    return seen


@pytest.mark.asyncio
@pytest.mark.performance
@pytest.mark.slow
async def test_recursive_cte_beats_n_plus_one(maybe_pool):
    pool: AsyncConnectionPool = maybe_pool
    root, total_links = await _seed_links(pool)

    graph = AsyncExecutionLinkGraph(connection_pool=pool)

    # N+1 traversal (baseline)
    t0 = time.perf_counter()
    seen_n1 = await _n_plus_one_traversal(pool, root)
    n1_duration = time.perf_counter() - t0

    # Recursive CTE traversal (optimized)
    t1 = time.perf_counter()
    subtree = await graph.get_subtree_batch(root_execution_id=root, max_depth=None)
    cte_duration = time.perf_counter() - t1

    # Validate counts (links vs nodes). Links exclude root; nodes include root.
    expected_nodes = total_links + 1  # add root
    assert len(seen_n1) == expected_nodes
    assert len(subtree) == total_links

    # Expect CTE to be faster than N+1; allow small margin for variance
    assert cte_duration < n1_duration
    # Log timings for visibility
    print(f"N+1 duration: {n1_duration*1000:.2f}ms, CTE duration: {cte_duration*1000:.2f}ms")
