import hashlib
import json
from typing import Any, List, Dict, Optional, Callable, cast, Literal
from concurrent.futures import ThreadPoolExecutor, as_completed

from ikam.ir.core import OpAST, OpType, fold_constants

from modelado.operators.core import (
    Operator,
    OperatorEnv,
    OperatorParams,
    record_provenance,
    ProvenanceRecord,
    MIME_STRUCTURED_DATA,
    MIME_TEXT,
    branch_child_env,
)
from modelado.environment_scope import add_scope_qualifiers
from modelado.plans.amendments import apply_plan_patch


class VerifyOperator(Operator):
    """
    Compares reconstructed bytes with original source bytes and returns a drift map if they differ.

    Parameters:
        - original_bytes: bytes - The expected bytes.
        - reconstructor: Callable[[Any], bytes] | Callable[[Any, OperatorEnv], bytes]
          Function that reconstructs the fragment to bytes. Required — there is no
          generic default because reconstruction is domain-specific and a wrong default
          would produce silent always-drift.
    """

    def apply(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> Dict[str, Any]:
        original_bytes = params.parameters.get("original_bytes")
        reconstructor = params.parameters.get("reconstructor")

        if not isinstance(original_bytes, bytes):
            raise ValueError("VerifyOperator requires 'original_bytes' (bytes) parameter")

        if not callable(reconstructor):
            raise ValueError(
                "VerifyOperator requires a callable 'reconstructor' parameter. "
                "There is no generic default — pass a function that converts your "
                "fragment type to bytes."
            )

        # Try env-aware signature first, fall back to fragment-only
        try:
            reconstructed_bytes = cast(Callable[[Any, OperatorEnv], bytes], reconstructor)(fragment, env)
        except TypeError:
            reconstructed_bytes = cast(Callable[[Any], bytes], reconstructor)(fragment)

        if not isinstance(reconstructed_bytes, bytes):
            raise TypeError(
                f"reconstructor must return bytes, got {type(reconstructed_bytes).__name__}"
            )

        status: Literal["success", "drift"] = "success"
        drift: Optional[Dict[str, Any]] = None

        if reconstructed_bytes != original_bytes:
            status = "drift"
            drift = {
                "size_diff": len(reconstructed_bytes) - len(original_bytes),
                "original_size": len(original_bytes),
                "reconstructed_size": len(reconstructed_bytes),
                "mismatches": [],
            }

            min_len = min(len(reconstructed_bytes), len(original_bytes))
            for i in range(min_len):
                if reconstructed_bytes[i] != original_bytes[i]:
                    mismatch: Dict[str, Any] = {
                        "offset": i,
                        "expected": int(original_bytes[i]),
                        "actual": int(reconstructed_bytes[i]),
                        "context_expected": original_bytes[max(0, i - 5):i + 5].hex(),
                        "context_actual": reconstructed_bytes[max(0, i - 5):i + 5].hex(),
                    }
                    drift["mismatches"].append(mismatch)
                    if len(drift["mismatches"]) >= 10:
                        break

            if not drift["mismatches"] and len(reconstructed_bytes) != len(original_bytes):
                drift["mismatches"].append({
                    "offset": min_len,
                    "type": "size_mismatch",
                    "expected_remaining": original_bytes[min_len:].hex() if min_len < len(original_bytes) else None,
                    "actual_remaining": reconstructed_bytes[min_len:].hex() if min_len < len(reconstructed_bytes) else None,
                })

        return {"status": status, "drift": drift}

    def provenance(self, params: OperatorParams, env: OperatorEnv) -> ProvenanceRecord:
        return record_provenance(params, env)


class ResolveOperator(Operator):
    """
    Fetches a fragment by ID (via loader) or by Slot name, and optionally
    injects it into a target slot.

    Parameters:
        - fragment_id: str (optional) - ID to fetch via loader.
        - slot_name: str (optional) - Name of an existing slot to resolve from.
        - target_slot: str (optional) - Name of the slot to inject the result into.
    """

    def apply(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> Any:
        fragment_id = params.parameters.get("fragment_id")
        slot_name = params.parameters.get("slot_name")
        target_slot = params.parameters.get("target_slot")

        result = None
        if fragment_id:
            if not env.loader:
                raise ValueError(
                    "ResolveOperator requires a loader in env when fragment_id is provided"
                )
            result = env.loader.load(cast(str, fragment_id))
        elif slot_name:
            if slot_name not in env.slots:
                raise ValueError(f"Slot '{slot_name}' not found in env")
            result = env.slots[slot_name]
        else:
            # If neither provided, default to input fragment
            result = fragment

        if target_slot:
            env.slots[cast(str, target_slot)] = result

        return result

    def provenance(self, params: OperatorParams, env: OperatorEnv) -> ProvenanceRecord:
        return record_provenance(params, env)


class JoinOperator(Operator):
    """
    Fans-in and merges a sequence of fragments into a single aggregate fragment.

    Parameters:
        - strategy: Literal["list", "dict", "concat"] - How to merge.
        - target_key: str (optional) - For 'dict' strategy.
    """

    def apply(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> Any:
        if not isinstance(fragment, list):
            raise ValueError("JoinOperator requires a list as input fragment")

        strategy = params.parameters.get("strategy", "list")

        if strategy == "list":
            return fragment
        elif strategy == "concat":
            if all(isinstance(x, str) for x in fragment):
                return "".join(fragment)
            elif all(isinstance(x, bytes) for x in fragment):
                return b"".join(fragment)
            else:
                raise ValueError("Concat strategy requires all items to be str or bytes")
        elif strategy == "dict":
            result = {}
            for item in fragment:
                if isinstance(item, dict):
                    result.update(item)
                else:
                    raise ValueError("Dict strategy requires all items to be dictionaries")
            return result
        else:
            raise ValueError(f"Unsupported join strategy: {strategy}")

    def provenance(self, params: OperatorParams, env: OperatorEnv) -> ProvenanceRecord:
        return record_provenance(params, env)


class EvalOperator(Operator):
    """
    Executes an ExpressionIR AST against bound inputs (Stage 3 Capability).

    Supports: arithmetic (ADD, SUB, MUL, DIV), aggregations (AGG_SUM, AGG_AVG,
    AGG_MAX, AGG_MIN, AGG_COUNT), LOAD (literal), REF (variable lookup).

    Parameters:
        - expression: Dict[str, Any] - OpAST-shaped dict with 'op_type' and 'operands'.
        - inputs: Dict[str, Any] - Variable bindings keyed by name (for REF nodes).
        - emit_edges: bool (optional) - If True, emit calculates/feeds edge records
          in env.slots["eval_edges"] for HugeGraph derivation tracing.
    """

    def apply(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> Any:
        expression = params.parameters.get("expression")
        if not expression:
            raise ValueError("EvalOperator requires 'expression' parameter")

        inputs: Dict[str, Any] = params.parameters.get("inputs") or {}
        emit_edges: bool = bool(params.parameters.get("emit_edges", False))

        # Accept raw dict or OpAST
        if isinstance(expression, dict):
            ast = OpAST.model_validate(expression)
        elif isinstance(expression, OpAST):
            ast = expression
        else:
            raise ValueError("'expression' must be a dict or OpAST instance")

        # Fold constants before evaluation (deterministic reduction)
        ast = fold_constants(ast)

        edges: List[Dict[str, Any]] = []
        result = self._eval(ast, inputs, edges)

        if emit_edges and edges:
            # Tag each edge with the operator's env_scope (D19/I2).
            # Flush to HugeGraph is deferred to the COMMIT step; operators only buffer.
            tagged_edges = [
                add_scope_qualifiers(properties=e, scope=env.env_scope)
                for e in edges
            ]
            env.slots["eval_edges"] = tagged_edges

        return {"result": result, "edges": edges if emit_edges else []}

    def _eval(self, ast: Any, inputs: Dict[str, Any], edges: List[Dict[str, Any]]) -> Any:
        """Recursively evaluate an OpAST node."""
        if not isinstance(ast, OpAST):
            return ast  # raw literal (int, float, str, etc.)

        op = ast.op_type

        def resolve(operand: Any) -> Any:
            if hasattr(operand, "op_type"):
                return self._eval(operand, inputs, edges)
            return operand  # literal int/float/str

        operands = [resolve(o) for o in (ast.operands or [])]

        if op == OpType.LOAD:
            return operands[0] if operands else ast.params.get("value")

        if op == OpType.REF:
            name = ast.params.get("name") or (operands[0] if operands else None)
            if name not in inputs:
                raise KeyError(f"EvalOperator: unbound variable '{name}'")
            val = inputs[name]
            edges.append({"type": "feeds", "from": name, "to": "result"})
            return val

        # Arithmetic
        if op == OpType.ADD:
            result = sum(operands)
            edges.append({"type": "calculates", "op": "ADD", "operands": operands, "result": result})
            return result
        if op == OpType.SUB:
            if len(operands) != 2:
                raise ValueError("SUB requires exactly 2 operands")
            result = operands[0] - operands[1]
            edges.append({"type": "calculates", "op": "SUB", "operands": operands, "result": result})
            return result
        if op == OpType.MUL:
            result = 1
            for v in operands:
                result *= v
            edges.append({"type": "calculates", "op": "MUL", "operands": operands, "result": result})
            return result
        if op == OpType.DIV:
            if len(operands) != 2:
                raise ValueError("DIV requires exactly 2 operands")
            if operands[1] == 0:
                raise ZeroDivisionError("EvalOperator: division by zero")
            result = operands[0] / operands[1]
            edges.append({"type": "calculates", "op": "DIV", "operands": operands, "result": result})
            return result

        # Aggregations (operands may be a single list or spread values)
        values = operands[0] if (len(operands) == 1 and isinstance(operands[0], list)) else operands
        if op == OpType.AGG_SUM:
            result = sum(values)
        elif op == OpType.AGG_AVG:
            if not values:
                raise ValueError("AGG_AVG requires at least one operand")
            result = sum(values) / len(values)
        elif op == OpType.AGG_MAX:
            result = max(values)
        elif op == OpType.AGG_MIN:
            result = min(values)
        elif op == OpType.AGG_COUNT:
            result = len(values)
        else:
            raise NotImplementedError(f"EvalOperator: unsupported op '{op}'")

        edges.append({"type": "calculates", "op": str(op), "operands": list(values), "result": result})
        return result

    def provenance(self, params: OperatorParams, env: OperatorEnv) -> ProvenanceRecord:
        return record_provenance(params, env)


class ApplyOperator(Operator):

    """
    Executes deterministic Delta patch algebra on StructuredData or Text.

    Parameters:
        - delta: List[Dict[str, Any]] - Patch operations.
        - delta_type: Literal["structured", "text"] - Type of patch.
        - base_hash: str (optional) - Expected hash of input before applying.
    """

    def apply(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> Any:
        delta = params.parameters.get("delta")
        delta_type = params.parameters.get("delta_type", "structured")
        base_hash = params.parameters.get("base_hash")
        rebase_over = params.parameters.get("rebase_over")

        if base_hash:
            current_hash = self._compute_hash(fragment, delta_type)
            if current_hash != base_hash:
                raise ValueError(
                    f"Base hash mismatch: expected {base_hash}, got {current_hash}"
                )

        if delta is None:
            return fragment

        if delta_type == "structured":
            if not isinstance(fragment, dict):
                raise ValueError("Structured patching requires a dictionary fragment")
            return apply_plan_patch(fragment, cast(List[Dict[str, Any]], delta))

        elif delta_type == "text":
            if not isinstance(fragment, str):
                raise ValueError("Text patching requires a string fragment")
            text_ops = cast(List[Dict[str, Any]], delta)
            if rebase_over is not None:
                text_ops = self._rebase_text_ops(
                    text_ops,
                    cast(List[Dict[str, Any]], rebase_over),
                )
            return self._apply_text_patch(fragment, text_ops)

        else:
            raise ValueError(f"Unsupported delta_type: {delta_type}")

    def _compute_hash(self, fragment: Any, delta_type: str) -> str:
        if delta_type == "structured":
            blob = json.dumps(fragment, sort_keys=True, separators=(",", ":")).encode(
                "utf-8"
            )
        else:
            blob = str(fragment).encode("utf-8")
        return hashlib.blake2b(blob, digest_size=16).hexdigest()

    def _apply_text_patch(self, text: str, ops: List[Dict[str, Any]]) -> str:
        # Sort ops by offset descending so high-offset ops are applied first,
        # keeping lower-offset positions stable.  Overlapping regions in the
        # original text are rejected as a data-integrity error.
        sorted_ops = sorted(ops, key=lambda x: x.get("at", 0), reverse=True)

        # Detect overlapping original-text regions before applying anything.
        # An "insert" has zero width in the original; a "delete" covers [at, at+length).
        covered: List[tuple] = []
        for op in sorted_ops:
            op_kind = op.get("op")
            at = op.get("at", 0)
            length = op.get("length", 0) if op_kind == "delete" else 0
            end = at + length
            for prev_at, prev_end in covered:
                if at < prev_end and end > prev_at:
                    raise ValueError(
                        f"Overlapping text patch ops at offsets [{at},{end}) "
                        f"and [{prev_at},{prev_end})"
                    )
            covered.append((at, end))

        current_text = text
        for op in sorted_ops:
            op_kind = op.get("op")
            at = op.get("at", 0)

            if op_kind == "insert":
                new_content = op.get("text", "")
                current_text = current_text[:at] + new_content + current_text[at:]
            elif op_kind == "delete":
                length = op.get("length", 0)
                current_text = current_text[:at] + current_text[at + length:]
            else:
                raise ValueError(f"Unsupported text patch op: {op_kind}")

        return current_text

    def _rebase_text_ops(
        self,
        ops: List[Dict[str, Any]],
        base_ops: List[Dict[str, Any]],
    ) -> List[Dict[str, Any]]:
        if not base_ops:
            return list(ops)

        rebased: List[Dict[str, Any]] = []
        for op in ops:
            op_kind = str(op.get("op"))
            at = int(op.get("at", 0))
            length = int(op.get("length", 0)) if op_kind == "delete" else 0

            for base_op in base_ops:
                base_kind = str(base_op.get("op"))
                base_at = int(base_op.get("at", 0))

                if base_kind == "insert":
                    inserted = str(base_op.get("text", ""))
                    if base_at <= at:
                        at += len(inserted)
                    continue

                if base_kind == "delete":
                    base_len = int(base_op.get("length", 0))
                    base_end = base_at + base_len

                    if op_kind == "insert":
                        if base_at < at < base_end:
                            raise ValueError("Rebase conflict: insert lands in deleted range")
                        if base_end <= at:
                            at -= base_len
                        continue

                    # op_kind == delete
                    op_end = at + length
                    if at < base_end and op_end > base_at:
                        raise ValueError("Rebase conflict: overlapping delete ranges")
                    if base_end <= at:
                        at -= base_len
                    continue

                raise ValueError(f"Unsupported rebase op: {base_kind}")

            updated = dict(op)
            updated["at"] = at
            rebased.append(updated)

        return rebased

    def provenance(self, params: OperatorParams, env: OperatorEnv) -> ProvenanceRecord:
        return record_provenance(params, env)


class MapOperator(Operator):
    """
    Dispatches concurrent tasks for each item in a sequence using a provided operator.

    Parameters:
        - operator: Operator - The operator to apply to each item.
        - child_params: OperatorParams - The parameters for the child operator.
        - max_workers: int (optional) - Maximum number of concurrent tasks. Default 4.
    """

    def apply(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> List[Any]:
        if not isinstance(fragment, list):
            raise ValueError("MapOperator requires a list as input fragment")

        child_operator = params.parameters.get("operator")
        child_params = params.parameters.get("child_params")
        max_workers = params.parameters.get("max_workers", 4)

        if not isinstance(child_operator, Operator) or not isinstance(
            child_params, OperatorParams
        ):
            raise ValueError("MapOperator requires 'operator' and 'child_params' parameters")

        results: List[Any] = [None] * len(fragment)

        with ThreadPoolExecutor(max_workers=cast(int, max_workers)) as executor:
            future_to_index = {
                executor.submit(
                    child_operator.apply, item, child_params, branch_child_env(env, i)
                ): i
                for i, item in enumerate(fragment)
            }

            for future in as_completed(future_to_index):
                index = future_to_index[future]
                try:
                    results[index] = future.result()
                except Exception as e:
                    raise e

        return results

    def provenance(self, params: OperatorParams, env: OperatorEnv) -> ProvenanceRecord:
        return record_provenance(params, env)
