from __future__ import annotations

import time
import uuid
from typing import Any, Dict, List, Optional, Tuple, Literal, cast

from ikam.graph import _cas_hex
from modelado.plans.schema import (
    PetriNetEnvelope,
    PetriNetMarking,
    PetriNetTransition,
    PetriNetRunFiring,
    PlanTransitionRef,
    canonicalize_petri_net_marking_json,
)
from modelado.operators.core import Operator, OperatorEnv, OperatorParams
from modelado.environment_scope import EnvironmentScope


class PetriNetEngine:
    """
    Manages transition firing and state transitions for Petri Net ingestion runs.
    """

    def __init__(
        self,
        net_envelope: PetriNetEnvelope,
        transitions: Dict[str, PetriNetTransition],
        env: OperatorEnv,
        net_artifact_id: Optional[str] = None,
    ):
        self.net_envelope = net_envelope
        self.transitions = transitions
        self.env = env
        self.net_artifact_id = net_artifact_id or "unknown-net"
        self._validate_env_scope()

    def _validate_env_scope(self) -> None:
        scope = getattr(self.env, "env_scope", None)
        if not isinstance(scope, EnvironmentScope):
            raise ValueError("PetriNetEngine requires env_scope to be set before any operator fires")

    def is_enabled(self, transition_id: str, marking: PetriNetMarking) -> bool:
        """Check if a transition is enabled in the current marking."""
        transition = self.transitions.get(transition_id)
        if not transition:
            return False

        # 1. Check input tokens
        for input_arc in transition.inputs:
            if marking.tokens.get(input_arc.place_id, 0) < input_arc.weight:
                return False

        # 2. Check retry budget if applicable
        # Transitions with 'repair' or 'retry' tags consume from the budget
        if any(tag in transition.tags for tag in ["repair", "retry"]):
            budget = marking.meta.get("retry_budget", 0)
            if budget <= 0:
                return False

        # 3. Check guard conditions (Stage 1 implementation: basic equality)
        if transition.guard:
            for key, expected_value in transition.guard.items():
                # Check against environment or marking meta
                actual_value = self.env.slots.get(key) or marking.meta.get(key)
                if actual_value != expected_value:
                    return False

        return True

    def _hash_marking(self, marking: PetriNetMarking) -> str:
        return _cas_hex(canonicalize_petri_net_marking_json(marking))

    def fire(
        self,
        transition_id: str,
        marking: PetriNetMarking,
        fragment: Any = None,
        params: Optional[OperatorParams] = None,
        transition_fragment_id: Optional[str] = None,
    ) -> Tuple[PetriNetMarking, PetriNetRunFiring]:
        """
        Execute a transition firing.

        Consumes tokens from input places, executes the operation, and produces tokens into output places.
        Returns the updated marking and the firing record.
        """
        transition = self.transitions.get(transition_id)
        if not transition:
            raise ValueError(f"Transition '{transition_id}' not found in net")

        self._validate_env_scope()

        if not self.is_enabled(transition_id, marking):
            raise ValueError(
                f"Transition '{transition_id}' is not enabled in current marking"
            )

        # 1. Capture marking before
        marking_before_id = self._hash_marking(marking)

        # 2. Consume tokens and update meta
        new_tokens = dict(marking.tokens)
        new_meta = dict(marking.meta)

        for input_arc in transition.inputs:
            new_tokens[input_arc.place_id] -= input_arc.weight
            if new_tokens[input_arc.place_id] <= 0:
                del new_tokens[input_arc.place_id]

        # Consume retry budget if applicable
        if any(tag in transition.tags for tag in ["repair", "retry"]):
            budget = new_meta.get("retry_budget", 0)
            new_meta["retry_budget"] = max(0, budget - 1)

        # 3. Execute operation
        status: Literal["success", "failed"] = "success"
        error_msg: Optional[str] = None
        effects: Dict[str, Any] = {}
        
        # Data hand-off logic
        input_key = transition.metadata.get("input_key", "current_fragment")
        output_key = transition.metadata.get("output_key", "current_fragment")
        params_key = transition.metadata.get("params_key")

        # Resolve input fragment
        actual_fragment = fragment
        if actual_fragment is None:
            actual_fragment = marking.meta.get(input_key)

        if transition_fragment_id:
            # Phase 3: runtime execution dispatch
            from modelado.executors import ExecutionDispatcher
            dispatcher = ExecutionDispatcher()
            previous_marking = self.env.slots.get("current_marking")
            self.env.slots["current_marking"] = marking
            
            # Merge params
            base_params = transition.metadata.get("params", {})
            meta_params = marking.meta.get(params_key, {}) if params_key else {}
            param_map = transition.metadata.get("param_map", {})
            mapped_params = {
                target: marking.meta.get(source) 
                for target, source in param_map.items() 
                if source in marking.meta
            }
            merged_parameters = {**base_params, **meta_params, **mapped_params}
            if params:
                merged_parameters.update(params.parameters)
                
            actual_params = OperatorParams(
                name=transition_id,
                parameters=merged_parameters,
            )
            
            if self.env.debug_sink:
                self.env.debug_sink.emit("invoke.start", {
                    "transition_id": transition_id,
                    "transition_fragment_id": transition_fragment_id,
                    "params": merged_parameters,
                })

            try:
                execution_queue_bus = self.env.slots.get("execution_queue_bus")
                if execution_queue_bus is not None:
                    queued = dispatcher.publish_execution_queue_request(
                        transition_fragment_id=transition_fragment_id,
                        fragment=actual_fragment,
                        params=actual_params,
                        env=self.env,
                        bus=execution_queue_bus,
                    )
                    op_result = queued.model_dump(mode="json")
                    effects["result"] = op_result
                    new_meta[output_key] = op_result
                else:
                    # Compile and execute via graph sidecar
                    execution_response = dispatcher.compile_and_execute(
                        transition_fragment_id=transition_fragment_id,
                        fragment=actual_fragment,
                        params=actual_params,
                        env=self.env
                    )

                    op_result = execution_response.get("result")
                    effects["result"] = op_result
                    new_meta[output_key] = op_result

                    # Apply context mutations from the sidecar
                    mutations = execution_response.get("context_mutations")
                    if mutations:
                        # Apply meta updates
                        for k, v in mutations.get("meta_updates", {}).items():
                            new_meta[k] = v
                        # Apply meta deletes
                        for k in mutations.get("meta_deletes", []):
                            new_meta.pop(k, None)
                        # Apply token deltas
                        for k, v in mutations.get("token_deltas", {}).items():
                            new_tokens[k] = new_tokens.get(k, 0) + v
                            if new_tokens[k] <= 0:
                                del new_tokens[k]
                            
                if self.env.debug_sink:
                    self.env.debug_sink.emit("invoke.ok", {
                        "transition_id": transition_id,
                        "transition_fragment_id": transition_fragment_id,
                    })
            except ValueError:
                raise
            except Exception as e:
                status = "failed"
                error_msg = str(e)
                if self.env.debug_sink:
                    self.env.debug_sink.emit("invoke.error", {
                        "transition_id": transition_id,
                        "error": error_msg,
                    })
            finally:
                if previous_marking is None:
                    self.env.slots.pop("current_marking", None)
                else:
                    self.env.slots["current_marking"] = previous_marking
        else:
            # If no operator, it's a structural transition (no-op)
            pass

        # 4. Produce tokens
        for output_arc in transition.outputs:
            new_tokens[output_arc.place_id] = (
                new_tokens.get(output_arc.place_id, 0) + output_arc.weight
            )

        provisional_marking = PetriNetMarking(tokens=new_tokens, meta=new_meta)

        enabled_next_transitions = [
            next_transition_id
            for next_transition_id in self.transitions
            if next_transition_id != transition_id and self.is_enabled(next_transition_id, provisional_marking)
        ]
        if enabled_next_transitions:
            new_meta["next_transition_ids"] = enabled_next_transitions
            new_meta["next_transition_id"] = enabled_next_transitions[0]
            effects["next_transition_ids"] = enabled_next_transitions
            effects["next_transition_id"] = enabled_next_transitions[0]

        new_marking = PetriNetMarking(tokens=new_tokens, meta=new_meta)

        marking_after_id = self._hash_marking(new_marking)

        # 5. Record firing
        firing = PetriNetRunFiring(
            firing_id=str(uuid.uuid4()),
            transition_ref=PlanTransitionRef(
                plan_artifact_id=self.net_artifact_id,
                transition_fragment_id=transition_fragment_id or "unknown-frag",
                transition_id=transition_id,
            ),
            marking_before_fragment_id=marking_before_id,
            marking_after_fragment_id=marking_after_id,
            status=status,
            error=error_msg,
            effects=effects,
            ts_ms=int(time.time() * 1000),
        )

        return new_marking, firing

def get_dynamic_execution_steps() -> list[str]:
    from modelado.db import connection_scope
    import json
    
    with connection_scope() as cx:
        with cx.cursor() as cur:
            cur.execute("""
                SELECT value FROM ikam_fragment_store 
                WHERE project_id = 'modelado/projects/canonical' 
                  AND value->>'profile' = 'petri_net'
                ORDER BY created_at DESC LIMIT 1
            """)
            row = cur.fetchone()
            if row and row["value"]: # type: ignore
                envelope = row["value"] # type: ignore
                if isinstance(envelope, str):
                    envelope = json.loads(envelope)
                
                data = envelope.get("data", {})
                section_cas_ids = data.get("section_cas_ids", [])
                
                all_transition_ids = []
                for sec_cas_id in section_cas_ids:
                    cur.execute("SELECT value FROM ikam_fragment_store WHERE cas_id = %s", (sec_cas_id,))
                    sec_row = cur.fetchone()
                    if sec_row and sec_row["value"]: # type: ignore
                        sec_val = sec_row["value"] # type: ignore
                        if isinstance(sec_val, str):
                            sec_val = json.loads(sec_val)
                        sec_data = sec_val.get("data", {})
                        t_ids = sec_data.get("transition_ids", [])
                        all_transition_ids.extend(t_ids)
                
                if all_transition_ids:
                    dynamic_steps = []
                    for t_id in all_transition_ids:
                        if t_id not in dynamic_steps:
                            dynamic_steps.append(t_id)
                            
                    if "map.conceptual.verify.discovery_gate" not in dynamic_steps:
                        dynamic_steps.append("map.conceptual.verify.discovery_gate")
                        
                    return dynamic_steps

    return [
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
