import hashlib
import json
from typing import Any, Dict

import pytest
from modelado.environment_scope import EnvironmentScope
from modelado.operators.core import OperatorEnv, OperatorParams, FragmentLoader, branch_child_env
from modelado.operators.monadic import (
    VerifyOperator,
    MapOperator,
    ResolveOperator,
    ApplyOperator,
    JoinOperator,
    EvalOperator,
)


_DEV_SCOPE = EnvironmentScope(ref="refs/heads/run/test")


def _compute_hash(fragment: Any, delta_type: str = "structured") -> str:
    if delta_type == "structured":
        blob = json.dumps(fragment, sort_keys=True, separators=(",", ":")).encode(
            "utf-8"
        )
    else:
        blob = str(fragment).encode("utf-8")
    return hashlib.blake2b(blob, digest_size=16).hexdigest()


def test_verify_operator_success():
    op = VerifyOperator()
    env = OperatorEnv(seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)

    original_bytes = b"hello world"
    # Reconstructor returns the same bytes
    reconstructor = lambda f: f

    params = OperatorParams(
        name="verify",
        parameters={
            "original_bytes": original_bytes,
            "reconstructor": reconstructor,
        },
    )

    result = op.apply(b"hello world", params, env)
    assert result["status"] == "success"
    assert result["drift"] is None


def test_verify_operator_drift():
    op = VerifyOperator()
    env = OperatorEnv(seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)

    original_bytes = b"hello world"
    # Reconstructor returns different bytes
    reconstructor = lambda f: f

    params = OperatorParams(
        name="verify",
        parameters={
            "original_bytes": original_bytes,
            "reconstructor": reconstructor,
        },
    )

    result = op.apply(b"hello world drifted", params, env)
    assert result["status"] == "drift"
    assert result["drift"]["size_diff"] == 8
    assert result["drift"]["original_size"] == 11
    assert result["drift"]["reconstructed_size"] == 19
    
    # Check enhanced mismatch structure
    mismatches = result["drift"]["mismatches"]
    assert len(mismatches) == 1
    assert mismatches[0]["offset"] == 11
    assert mismatches[0]["type"] == "size_mismatch"
    assert mismatches[0]["actual_remaining"] == b" drifted".hex()
    assert mismatches[0]["expected_remaining"] is None


def test_verify_operator_byte_mismatch():
    op = VerifyOperator()
    env = OperatorEnv(seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)

    original_bytes = b"hello world"
    # Reconstructor returns bytes with one mismatch at index 4 ('o' vs 'x')
    reconstructed = b"hellx world"
    reconstructor = lambda f: f

    params = OperatorParams(
        name="verify",
        parameters={
            "original_bytes": original_bytes,
            "reconstructor": reconstructor,
        },
    )

    result = op.apply(reconstructed, params, env)
    assert result["status"] == "drift"
    
    mismatches = result["drift"]["mismatches"]
    assert len(mismatches) == 1
    assert mismatches[0]["offset"] == 4
    assert mismatches[0]["expected"] == ord('o')
    assert mismatches[0]["actual"] == ord('x')
    assert "context_expected" in mismatches[0]
    assert "context_actual" in mismatches[0]


def test_resolve_operator_loader():
    class MockLoader(FragmentLoader):
        def load(self, fragment_id: str) -> Any:
            return f"fragment-{fragment_id}"

    op = ResolveOperator()
    env = OperatorEnv(
        seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE,
        loader=MockLoader()
    )

    params = OperatorParams(
        name="resolve", parameters={"fragment_id": "123", "target_slot": "base"}
    )

    result = op.apply(None, params, env)
    assert result == "fragment-123"
    assert env.slots["base"] == "fragment-123"


def test_resolve_operator_slot():
    op = ResolveOperator()
    env = OperatorEnv(seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)
    env.slots["input"] = "something"

    params = OperatorParams(name="resolve", parameters={"slot_name": "input"})

    result = op.apply(None, params, env)
    assert result == "something"


def test_apply_operator_structured():
    op = ApplyOperator()
    env = OperatorEnv(seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)

    fragment = {"a": 1, "b": {"c": 2}}
    delta = [{"op": "replace", "path": "/b/c", "value": 3}, {"op": "add", "path": "/d", "value": 4}]

    params = OperatorParams(
        name="apply", parameters={"delta": delta, "delta_type": "structured"}
    )

    result = op.apply(fragment, params, env)
    assert result == {"a": 1, "b": {"c": 3}, "d": 4}


def test_apply_operator_with_base_hash():
    op = ApplyOperator()
    env = OperatorEnv(seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)

    fragment = {"a": 1, "b": {"c": 2}}
    delta = [{"op": "replace", "path": "/b/c", "value": 3}]

    # Correct base_hash
    base_hash = _compute_hash(fragment)
    params = OperatorParams(
        name="apply", parameters={"delta": delta, "base_hash": base_hash}
    )
    result = op.apply(fragment, params, env)
    assert result["b"]["c"] == 3

    # Incorrect base_hash
    bad_params = OperatorParams(
        name="apply", parameters={"delta": delta, "base_hash": "badhash"}
    )
    with pytest.raises(ValueError, match="Base hash mismatch"):
        op.apply(fragment, bad_params, env)


def test_join_operator_list():
    op = JoinOperator()
    env = OperatorEnv(seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)
    items = [1, 2, 3]
    params = OperatorParams(name="join", parameters={"strategy": "list"})
    assert op.apply(items, params, env) == [1, 2, 3]


def test_join_operator_concat():
    op = JoinOperator()
    env = OperatorEnv(seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)
    items = ["a", "b", "c"]
    params = OperatorParams(name="join", parameters={"strategy": "concat"})
    assert op.apply(items, params, env) == "abc"


def test_join_operator_dict():
    op = JoinOperator()
    env = OperatorEnv(seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)
    items = [{"a": 1}, {"b": 2}]
    params = OperatorParams(name="join", parameters={"strategy": "dict"})
    assert op.apply(items, params, env) == {"a": 1, "b": 2}


def test_eval_operator_invalid_expression_raises():
    """Dict that cannot be validated as OpAST raises ValidationError."""
    from pydantic import ValidationError
    op = EvalOperator()
    env = OperatorEnv(seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)
    # Missing required 'op_type' field → Pydantic ValidationError
    expression = {"op": "add", "args": [1, 2]}
    params = OperatorParams(name="eval", parameters={"expression": expression})
    with pytest.raises(ValidationError):
        op.apply(None, params, env)


def test_apply_operator_text():
    op = ApplyOperator()
    env = OperatorEnv(seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)

    fragment = "hello world"
    delta = [
        {"op": "insert", "at": 6, "text": "beautiful "},
        {"op": "delete", "at": 0, "length": 5},
    ]

    params = OperatorParams(
        name="apply", parameters={"delta": delta, "delta_type": "text"}
    )

    result = op.apply(fragment, params, env)
    assert result == " beautiful world"


def test_map_operator_success():
    class DoubleOperator:
        def apply(self, fragment: int, params: OperatorParams, env: OperatorEnv) -> int:
            return fragment * 2

        def provenance(self, params: OperatorParams, env: OperatorEnv) -> Any:
            return None

    op = MapOperator()
    env = OperatorEnv(seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)

    params = OperatorParams(
        name="map",
        parameters={
            "operator": DoubleOperator(),
            "child_params": OperatorParams(name="double", parameters={}),
            "max_workers": 2,
        },
    )

    items = [1, 2, 3, 4, 5]
    results = op.apply(items, params, env)
    assert results == [2, 4, 6, 8, 10]


def test_map_operator_empty():
    class NoopOperator:
        def apply(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> Any:
            return fragment

        def provenance(self, params: OperatorParams, env: OperatorEnv) -> Any:
            return None

    op = MapOperator()
    env = OperatorEnv(seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)

    params = OperatorParams(
        name="map",
        parameters={
            "operator": NoopOperator(),
            "child_params": OperatorParams(name="noop", parameters={}),
        },
    )

    items: list[Any] = []
    results = op.apply(items, params, env)
    assert results == []


def test_map_operator_error():
    class ErrorOperator:
        def apply(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> Any:
            raise ValueError("boom")

        def provenance(self, params: OperatorParams, env: OperatorEnv) -> Any:
            return None

    op = MapOperator()
    env = OperatorEnv(seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)

    params = OperatorParams(
        name="map",
        parameters={
            "operator": ErrorOperator(),
            "child_params": OperatorParams(name="error", parameters={}),
        },
    )

    items = [1]
    with pytest.raises(ValueError, match="boom"):
        op.apply(items, params, env)


# ---------------------------------------------------------------------------
# EvalOperator — Stage 3 numeric round-trip tests
# ---------------------------------------------------------------------------

def _env() -> OperatorEnv:
    return OperatorEnv(seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)


def test_eval_load_literal():
    """LOAD node with a literal value returns that value."""
    op = EvalOperator()
    expr = {"op_type": "LOAD", "operands": [42], "params": {}}
    result = op.apply(None, OperatorParams(name="eval", parameters={"expression": expr}), _env())
    assert result["result"] == 42


def test_eval_ref_binding():
    """REF node resolves a named variable from inputs."""
    op = EvalOperator()
    expr = {"op_type": "REF", "operands": [], "params": {"name": "revenue"}}
    result = op.apply(
        None,
        OperatorParams(name="eval", parameters={"expression": expr, "inputs": {"revenue": 1_000_000}}),
        _env(),
    )
    assert result["result"] == 1_000_000


def test_eval_add():
    """ADD of two literals."""
    op = EvalOperator()
    expr = {
        "op_type": "ADD",
        "operands": [
            {"op_type": "LOAD", "operands": [3], "params": {}},
            {"op_type": "LOAD", "operands": [4], "params": {}},
        ],
        "params": {},
    }
    result = op.apply(None, OperatorParams(name="eval", parameters={"expression": expr}), _env())
    assert result["result"] == 7


def test_eval_sub():
    op = EvalOperator()
    expr = {
        "op_type": "SUB",
        "operands": [
            {"op_type": "LOAD", "operands": [10], "params": {}},
            {"op_type": "LOAD", "operands": [3], "params": {}},
        ],
        "params": {},
    }
    result = op.apply(None, OperatorParams(name="eval", parameters={"expression": expr}), _env())
    assert result["result"] == 7


def test_eval_mul():
    op = EvalOperator()
    expr = {
        "op_type": "MUL",
        "operands": [
            {"op_type": "LOAD", "operands": [6], "params": {}},
            {"op_type": "LOAD", "operands": [7], "params": {}},
        ],
        "params": {},
    }
    result = op.apply(None, OperatorParams(name="eval", parameters={"expression": expr}), _env())
    assert result["result"] == 42


def test_eval_div():
    op = EvalOperator()
    expr = {
        "op_type": "DIV",
        "operands": [
            {"op_type": "LOAD", "operands": [100.0], "params": {}},
            {"op_type": "LOAD", "operands": [4.0], "params": {}},
        ],
        "params": {},
    }
    result = op.apply(None, OperatorParams(name="eval", parameters={"expression": expr}), _env())
    assert result["result"] == 25.0


def test_eval_div_by_zero():
    op = EvalOperator()
    expr = {
        "op_type": "DIV",
        "operands": [
            {"op_type": "LOAD", "operands": [1], "params": {}},
            {"op_type": "LOAD", "operands": [0], "params": {}},
        ],
        "params": {},
    }
    with pytest.raises(ZeroDivisionError):
        op.apply(None, OperatorParams(name="eval", parameters={"expression": expr}), _env())


def test_eval_agg_sum():
    op = EvalOperator()
    expr = {
        "op_type": "AGG_SUM",
        "operands": [
            {"op_type": "LOAD", "operands": [1], "params": {}},
            {"op_type": "LOAD", "operands": [2], "params": {}},
            {"op_type": "LOAD", "operands": [3], "params": {}},
        ],
        "params": {},
    }
    result = op.apply(None, OperatorParams(name="eval", parameters={"expression": expr}), _env())
    assert result["result"] == 6


def test_eval_agg_avg():
    op = EvalOperator()
    expr = {
        "op_type": "AGG_AVG",
        "operands": [
            {"op_type": "LOAD", "operands": [10], "params": {}},
            {"op_type": "LOAD", "operands": [20], "params": {}},
        ],
        "params": {},
    }
    result = op.apply(None, OperatorParams(name="eval", parameters={"expression": expr}), _env())
    assert result["result"] == 15.0


def test_eval_agg_max():
    op = EvalOperator()
    expr = {
        "op_type": "AGG_MAX",
        "operands": [
            {"op_type": "LOAD", "operands": [5], "params": {}},
            {"op_type": "LOAD", "operands": [99], "params": {}},
            {"op_type": "LOAD", "operands": [3], "params": {}},
        ],
        "params": {},
    }
    result = op.apply(None, OperatorParams(name="eval", parameters={"expression": expr}), _env())
    assert result["result"] == 99


def test_eval_agg_count():
    op = EvalOperator()
    expr = {
        "op_type": "AGG_COUNT",
        "operands": [
            {"op_type": "LOAD", "operands": [1], "params": {}},
            {"op_type": "LOAD", "operands": [2], "params": {}},
            {"op_type": "LOAD", "operands": [3], "params": {}},
            {"op_type": "LOAD", "operands": [4], "params": {}},
        ],
        "params": {},
    }
    result = op.apply(None, OperatorParams(name="eval", parameters={"expression": expr}), _env())
    assert result["result"] == 4


def test_eval_nested_expression():
    """(revenue - cost) / units — simulates a unit-margin formula."""
    op = EvalOperator()
    expr = {
        "op_type": "DIV",
        "operands": [
            {
                "op_type": "SUB",
                "operands": [
                    {"op_type": "REF", "operands": [], "params": {"name": "revenue"}},
                    {"op_type": "REF", "operands": [], "params": {"name": "cost"}},
                ],
                "params": {},
            },
            {"op_type": "REF", "operands": [], "params": {"name": "units"}},
        ],
        "params": {},
    }
    inputs = {"revenue": 500_000.0, "cost": 200_000.0, "units": 1_000.0}
    result = op.apply(
        None,
        OperatorParams(name="eval", parameters={"expression": expr, "inputs": inputs}),
        _env(),
    )
    assert result["result"] == pytest.approx(300.0)


def test_eval_emit_edges_tagged_with_env_scope():
    """emit_edges=True records scope-tagged calculates and feeds edges in env.slots (D19/I2)."""
    op = EvalOperator()
    env = _env()
    expr = {
        "op_type": "ADD",
        "operands": [
            {"op_type": "REF", "operands": [], "params": {"name": "x"}},
            {"op_type": "LOAD", "operands": [1], "params": {}},
        ],
        "params": {},
    }
    result = op.apply(
        None,
        OperatorParams(name="eval", parameters={"expression": expr, "inputs": {"x": 9}, "emit_edges": True}),
        env,
    )
    assert result["result"] == 10
    assert "eval_edges" in env.slots
    edges = env.slots["eval_edges"]
    edge_types = {e["type"] for e in edges}
    assert "feeds" in edge_types
    assert "calculates" in edge_types
    # Edge qualifiers now expose only the canonical ref.
    for edge in edges:
        assert edge["ref"] == "refs/heads/run/test"


def test_eval_unbound_ref_raises():
    """REF with an unknown variable name raises KeyError."""
    op = EvalOperator()
    expr = {"op_type": "REF", "operands": [], "params": {"name": "missing_var"}}
    with pytest.raises(KeyError, match="missing_var"):
        op.apply(None, OperatorParams(name="eval", parameters={"expression": expr, "inputs": {}}), _env())


def test_eval_sum_div_count_formula():
    """OpAST formula SUM(values)/COUNT(values) == arithmetic average (not an Excel round-trip)."""
    op = EvalOperator()
    values = [10.0, 20.0, 30.0]

    # Simulate =SUM(values)/COUNT(values)
    load_nodes = [{"op_type": "LOAD", "operands": [v], "params": {}} for v in values]

    sum_expr = {"op_type": "AGG_SUM", "operands": load_nodes, "params": {}}
    count_expr = {"op_type": "AGG_COUNT", "operands": load_nodes, "params": {}}

    formula = {
        "op_type": "DIV",
        "operands": [sum_expr, count_expr],
        "params": {},
    }

    result = op.apply(None, OperatorParams(name="eval", parameters={"expression": formula}), _env())
    # Expected: (10+20+30)/3 = 20.0 — matches Excel's AVERAGE(A1:A3)
    assert result["result"] == pytest.approx(20.0)


def test_eval_agg_avg_empty_raises():
    """AGG_AVG with no operands raises ValueError."""
    op = EvalOperator()
    expr = {"op_type": "AGG_AVG", "operands": [], "params": {}}
    with pytest.raises(ValueError, match="AGG_AVG requires at least one operand"):
        op.apply(None, OperatorParams(name="eval", parameters={"expression": expr}), _env())


def test_apply_operator_text_overlap_raises():
    """Overlapping delete regions in a text patch raise ValueError."""
    op = ApplyOperator()
    env = OperatorEnv(seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)

    fragment = "hello world"
    # Two deletes that overlap: [2,7) and [5,8) share [5,7)
    delta = [
        {"op": "delete", "at": 2, "length": 5},
        {"op": "delete", "at": 5, "length": 3},
    ]
    params = OperatorParams(name="apply", parameters={"delta": delta, "delta_type": "text"})
    with pytest.raises(ValueError, match="Overlapping text patch ops"):
        op.apply(fragment, params, env)


def test_apply_operator_text_rebase_over_insert_shift():
    """Rebasing text ops over base inserts shifts offsets deterministically."""
    op = ApplyOperator()
    env = OperatorEnv(seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)

    # Original base was "hello world". Base evolved to "XXhello world" via insert at 0.
    fragment = "XXhello world"
    delta = [{"op": "insert", "at": 11, "text": "!"}]
    rebase_over = [{"op": "insert", "at": 0, "text": "XX"}]

    params = OperatorParams(
        name="apply",
        parameters={
            "delta": delta,
            "delta_type": "text",
            "rebase_over": rebase_over,
        },
    )

    result = op.apply(fragment, params, env)
    assert result == "XXhello world!"


def test_apply_operator_text_rebase_conflict_raises():
    """Rebasing over overlapping deletes fails explicitly."""
    op = ApplyOperator()
    env = OperatorEnv(seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)

    fragment = "abcdef"
    delta = [{"op": "delete", "at": 2, "length": 2}]
    rebase_over = [{"op": "delete", "at": 1, "length": 3}]

    params = OperatorParams(
        name="apply",
        parameters={
            "delta": delta,
            "delta_type": "text",
            "rebase_over": rebase_over,
        },
    )

    with pytest.raises(ValueError, match="Rebase conflict"):
        op.apply(fragment, params, env)


# ---------------------------------------------------------------------------
# D19 — branch_child_env isolation and scope-tagging tests
# ---------------------------------------------------------------------------

def test_branch_child_env_creates_isolated_slots():
    """Each child env from branch_child_env gets an independent slots dict (I9)."""
    parent = OperatorEnv(seed=1, renderer_version="v1", policy="default", env_scope=_DEV_SCOPE)
    child_0 = branch_child_env(parent, 0)
    child_1 = branch_child_env(parent, 1)

    child_0.slots["key"] = "from_child_0"
    child_1.slots["key"] = "from_child_1"

    # Children do not share slots with each other or the parent
    assert "key" not in parent.slots
    assert child_0.slots["key"] == "from_child_0"
    assert child_1.slots["key"] == "from_child_1"


def test_branch_child_env_derives_child_ref_from_parent_ref():
    """Child scope derives from parent ref."""
    parent = OperatorEnv(
        seed=1, renderer_version="v1", policy="default",
        env_scope=EnvironmentScope(ref="refs/heads/run/abc123"),
    )
    child = branch_child_env(parent, 3)

    assert child.env_scope.ref == "refs/heads/run/abc123/item-3"


def test_branch_child_env_supports_ref_parent_scope_inputs():
    parent = OperatorEnv(
        seed=1,
        renderer_version="v1",
        policy="default",
        env_scope=EnvironmentScope(ref="refs/heads/run/legacy-parent"),
    )

    child = branch_child_env(parent, 1)

    assert child.env_scope.ref == "refs/heads/run/legacy-parent/item-1"


def test_branch_child_env_inherits_parent_fields():
    """Child inherits seed, renderer_version, policy, loader, llm from parent."""
    parent = OperatorEnv(
        seed=99, renderer_version="v2", policy="relaxed", env_scope=_DEV_SCOPE,
        model_hash="abc", variation_id="v1",
    )
    child = branch_child_env(parent, 0)

    assert child.seed == 99
    assert child.renderer_version == "v2"
    assert child.policy == "relaxed"
    assert child.model_hash == "abc"
    assert child.variation_id == "v1"


def test_map_operator_child_envs_are_isolated():
    """MapOperator: concurrent items writing to their env.slots do not race (I9)."""
    import threading

    written_ids: list[str] = []
    lock = threading.Lock()

    class SlotWritingOperator:
        def apply(self, fragment: int, params: OperatorParams, env: OperatorEnv) -> int:
            env.slots["item_result"] = fragment * 10
            with lock:
                written_ids.append(env.env_scope.ref)
            return env.slots["item_result"]

        def provenance(self, params: OperatorParams, env: OperatorEnv) -> Any:
            return None

    op = MapOperator()
    parent_env = OperatorEnv(
        seed=1, renderer_version="v1", policy="default",
        env_scope=EnvironmentScope(ref="refs/heads/run/isolation-test"),
    )

    params = OperatorParams(
        name="map",
        parameters={
            "operator": SlotWritingOperator(),
            "child_params": OperatorParams(name="slot_writer", parameters={}),
            "max_workers": 4,
        },
    )

    results = op.apply([1, 2, 3, 4], params, parent_env)
    assert sorted(results) == [10, 20, 30, 40]

    # Parent env must not have been mutated
    assert "item_result" not in parent_env.slots

    # Each child got a unique, sub-scoped ref
    assert len(set(written_ids)) == 4
    for ref in written_ids:
        assert ref.startswith("refs/heads/run/isolation-test/item-")
