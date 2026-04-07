import copy
import json
import os
from typing import Any, Dict, Optional
import urllib.error
import urllib.request

from interacciones.schemas import ExecutionQueued, ExecutionQueueRequest
from modelado.core.execution_context import get_execution_context
from modelado.graph.preload_declarations import load_executor_declarations
from modelado.operators.core import OperatorParams, OperatorEnv
from modelado.db import connection_scope

class ExecutionDispatcher:
    """
    Traverses the core IKAM IR graphs to resolve Operator and Executor 
    configurations, then dispatches execution to an isolated Sidecar.
    """

    def __init__(self):
        pass

    def get_fragment_value(self, fragment_id: str) -> Optional[Dict[str, Any]]:
        with connection_scope() as cx:
            with cx.cursor() as cur:
                # Query by cas_id OR by the logical fragment_id injected into value
                cur.execute(
                    """
                    SELECT value FROM ikam_fragment_store 
                    WHERE cas_id = %s OR value->>'_fragment_id' = %s
                    LIMIT 1
                    """,
                    (fragment_id, fragment_id)
                )
                row = cur.fetchone()
                if row and row["value"]: # type: ignore
                    value = row["value"] # type: ignore
                    if (
                        isinstance(value, dict)
                        and value.get("ir_profile") == "StructuredDataIR"
                        and value.get("type") == "sidecar"
                        and os.getenv("IKAM_EXECUTOR_SIDECAR_URL")
                    ):
                        overridden = copy.deepcopy(value)
                        overridden["endpoint"] = os.getenv("IKAM_EXECUTOR_SIDECAR_URL")
                        return overridden
                    return value
        return None

    def get_object_for_predicate(self, subject_id: str, predicate: str) -> Optional[str]:
        with connection_scope() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    """
                    SELECT value->>'object' as obj 
                    FROM ikam_fragment_store 
                    WHERE value->>'ir_profile' = 'PropositionIR' 
                      AND value->>'subject' = %s 
                      AND value->>'predicate' = %s
                    """,
                    (subject_id, predicate)
                )
                row = cur.fetchone()
                if row and row["obj"]: # type: ignore
                    return row["obj"] # type: ignore
        return None

    def _resolve_operator_and_executor(self, transition_fragment_id: str) -> tuple[str, Dict[str, Any], str, Dict[str, Any]]:
        operator_id = self.get_object_for_predicate(transition_fragment_id, "executed_by_operator")
        if not operator_id:
            raise ValueError(f"No operator found for transition {transition_fragment_id}")

        expr_val = self.get_fragment_value(operator_id)
        if not expr_val or expr_val.get("ir_profile") != "ExpressionIR":
            raise ValueError(f"Operator {operator_id} is not a valid ExpressionIR")

        executor_id = self.get_object_for_predicate(operator_id, "executed_by")
        if not executor_id:
            raise ValueError(f"No executor found for operator {operator_id}")

        exec_val = self.get_fragment_value(executor_id)
        if not exec_val or exec_val.get("ir_profile") != "StructuredDataIR":
            raise ValueError(f"Executor {executor_id} is not a valid StructuredDataIR")

        return operator_id, expr_val, executor_id, exec_val

    def _build_context(self, env: OperatorEnv) -> Dict[str, Any]:
        marking = env.slots.get("current_marking")
        return {
            "tokens": marking.tokens if marking else {},
            "meta": marking.meta if marking else {},
            "env_scope": {"ref": env.env_scope.ref} if getattr(env, "env_scope", None) else {},
        }

    def _build_sidecar_payload(
        self,
        *,
        expr_val: Dict[str, Any],
        fragment: Any,
        params: OperatorParams,
        env: OperatorEnv,
    ) -> Dict[str, Any]:
        return {
            "module": expr_val.get("module"),
            "entrypoint": expr_val.get("entrypoint"),
            "payload": {
                "fragment": fragment,
                "params": params.parameters,
            },
            "context": self._build_context(env),
        }

    def _resolve_executor_declaration(self, expr_val: Dict[str, Any]):
        declarations = load_executor_declarations()
        params = expr_val.get("ast", {}).get("params", {})
        capability = params.get("capability")
        candidate_ids = []
        direct_executor_ref = params.get("direct_executor_ref")
        if isinstance(direct_executor_ref, str) and direct_executor_ref:
            candidate_ids.append(direct_executor_ref)
        eligible_executor_ids = params.get("eligible_executor_ids")
        if isinstance(eligible_executor_ids, list):
            candidate_ids.extend(
                executor_id for executor_id in eligible_executor_ids if isinstance(executor_id, str) and executor_id
            )

        seen: set[str] = set()
        for executor_id in candidate_ids:
            if executor_id in seen:
                continue
            seen.add(executor_id)
            for declaration in declarations:
                if declaration.executor_id == executor_id:
                    return declaration

        if isinstance(capability, str) and capability:
            for declaration in declarations:
                if capability in declaration.capabilities:
                    return declaration

        raise ValueError("Operator queue dispatch metadata does not resolve to a declared executor")

    def build_execution_queue_request(
        self,
        transition_fragment_id: str,
        fragment: Any,
        params: OperatorParams,
        env: OperatorEnv,
    ) -> ExecutionQueueRequest:
        _, expr_val, _, _ = self._resolve_operator_and_executor(transition_fragment_id)
        declaration = self._resolve_executor_declaration(expr_val)
        operator_params = expr_val.get("ast", {}).get("params", {})
        execution_context = get_execution_context()

        workflow_id = operator_params.get("workflow_id") or env.slots.get("workflow_id") or transition_fragment_id
        step_id = operator_params.get("transition_id") or params.name
        capability = operator_params.get("capability") or params.name
        policy = operator_params.get("policy") if isinstance(operator_params.get("policy"), dict) else {}
        constraints = operator_params.get("constraints") if isinstance(operator_params.get("constraints"), dict) else {}

        return ExecutionQueueRequest(
            request_id=(execution_context.request_id if execution_context else None) or transition_fragment_id,
            workflow_id=workflow_id,
            step_id=step_id,
            executor_id=declaration.executor_id,
            executor_kind=declaration.executor_kind,
            capability=capability,
            policy=policy,
            constraints=constraints,
            payload=self._build_sidecar_payload(expr_val=expr_val, fragment=fragment, params=params, env=env),
            transport=declaration.transport,
        )

    def publish_execution_queue_request(
        self,
        transition_fragment_id: str,
        fragment: Any,
        params: OperatorParams,
        env: OperatorEnv,
        *,
        bus: Any,
    ) -> ExecutionQueued:
        request = self.build_execution_queue_request(
            transition_fragment_id=transition_fragment_id,
            fragment=fragment,
            params=params,
            env=env,
        )
        bus.publish_request(request)
        return ExecutionQueued(
            request_id=request.request_id,
            workflow_id=request.workflow_id,
            step_id=request.step_id,
            executor_id=request.executor_id,
            executor_kind=request.executor_kind,
            capability=request.capability,
        )

    def compile_and_execute(
        self,
        transition_fragment_id: str,
        fragment: Any,
        params: OperatorParams,
        env: OperatorEnv
    ) -> Any:
        _, expr_val, executor_id, exec_val = self._resolve_operator_and_executor(transition_fragment_id)

        endpoint = exec_val.get("endpoint")
        if not endpoint:
            raise ValueError(f"Executor config {executor_id} missing 'endpoint'")

        payload = self._build_sidecar_payload(expr_val=expr_val, fragment=fragment, params=params, env=env)

        final_result = None

        try:
            request = urllib.request.Request(
                f"{endpoint}/execute",
                data=json.dumps(payload).encode("utf-8"),
                headers={"Content-Type": "application/json"},
                method="POST",
            )
            with urllib.request.urlopen(request) as response:
                for line in response.read().decode("utf-8").splitlines():
                    if line:
                        data = json.loads(line)
                        if data.get("type") == "trace":
                            sink = getattr(env, "debug_sink", None)
                            if sink is not None:
                                sink.emit(
                                    data.get("event_type", "sidecar.trace"),
                                    data.get("payload", {})
                                )
                        elif data.get("type") == "result":
                            if data.get("status") == "failed":
                                raise RuntimeError(f"Remote executor failed: {data.get('error')}")
                            final_result = data
                            break
                            
            if final_result is None:
                raise RuntimeError("Remote executor stream ended without yielding a result")
                
            return final_result
            
        except urllib.error.URLError as e:
            detail_str = str(getattr(e, "reason", e))
            raise RuntimeError(f"Failed to execute on remote sidecar: {detail_str}")
