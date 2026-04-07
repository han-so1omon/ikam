from __future__ import annotations
import hashlib
import json
from dataclasses import dataclass, field
import json
from typing import Protocol, Optional, Dict, Any, runtime_checkable, TYPE_CHECKING

from modelado.environment_scope import EnvironmentScope

if TYPE_CHECKING:
    from modelado.oraculo.ai_client import AIClient


@runtime_checkable
class DebugSink(Protocol):
    def emit(self, event_type: str, payload: Dict[str, Any]) -> None:
        ...


class InMemoryDebugSink:
    def __init__(self) -> None:
        self.events: list[Dict[str, Any]] = []

    def emit(self, event_type: str, payload: Dict[str, Any]) -> None:
        self.events.append({
            "event_type": event_type,
            "payload": payload,
        })


@runtime_checkable
class FragmentLoader(Protocol):
    def load(self, fragment_id: str) -> Any:
        ...


@dataclass(frozen=True)
class OperatorEnv:
    seed: int
    renderer_version: str
    policy: str
    env_scope: EnvironmentScope  # required (D19): no default, must be set before any operator fires
    model_hash: Optional[str] = None
    variation_id: Optional[str] = None
    slots: Dict[str, Any] = field(default_factory=dict)
    loader: Optional[FragmentLoader] = None
    history: Optional[Any] = None # Added for historical feedback (HistoricalFeedback type)
    llm: Optional["AIClient"] = None # Added for agentic operators (Plan C)
    debug_sink: Optional[DebugSink] = None # Added for monadic execution tracing


def branch_child_env(parent: OperatorEnv, item_index: int) -> OperatorEnv:
    """Create an isolated child OperatorEnv for parallel map branches (D19).

    The child derives a sub-scoped ref from the parent scope and gets a fresh
    slots dict, preventing concurrent writes from racing on shared state.
    """
    child_scope = EnvironmentScope(
        ref=f"{parent.env_scope.ref}/item-{item_index}",
    )
    return OperatorEnv(
        seed=parent.seed,
        renderer_version=parent.renderer_version,
        policy=parent.policy,
        env_scope=child_scope,
        model_hash=parent.model_hash,
        variation_id=parent.variation_id,
        slots={},  # isolated: no shared state with parent or siblings
        loader=parent.loader,
        history=parent.history,
        llm=parent.llm,
        debug_sink=parent.debug_sink,
    )


# Canonical MIME types for IKAM IR (Decision D8, D11)
MIME_STRUCTURED_DATA = "application/ikam-structured-data+json"
MIME_EXPRESSION = "application/ikam-expression+json"
MIME_PROPOSITION = "application/ikam-proposition+json"
MIME_RELATION = "application/ikam-relation+json"
MIME_STRUCTURAL_MAP = "application/ikam-structural-map+json"
MIME_TEXT = "text/plain"


@dataclass(frozen=True)
class OperatorParams:
    name: str
    parameters: Dict[str, Any]


@dataclass(frozen=True)
class OperatorDescriptor:
    operator: Any
    operator_ref: str
    capabilities: tuple[str, ...] = ()
    selection_policy: Any | None = None

    def __post_init__(self) -> None:
        try:
            json.dumps(self.selection_policy)
        except TypeError as exc:
            raise ValueError("selection_policy must be JSON-serializable") from exc


@dataclass(frozen=True)
class ProvenanceRecord:
    op_type: str
    params_hash: str
    seed: int
    renderer_version: str
    policy: str
    model_hash: Optional[str]
    variation_id: Optional[str]


@runtime_checkable
class Operator(Protocol):

    def apply(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> Any:
        ...

    def provenance(self, params: OperatorParams, env: OperatorEnv) -> ProvenanceRecord:
        ...


def params_hash(params: OperatorParams) -> str:
    import hashlib
    import json

    payload = {"name": params.name, "parameters": params.parameters}
    blob = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.blake2b(blob, digest_size=16).hexdigest()


def record_provenance(params: OperatorParams, env: OperatorEnv) -> ProvenanceRecord:
    return ProvenanceRecord(
        op_type=params.name,
        params_hash=params_hash(params),
        seed=env.seed,
        renderer_version=env.renderer_version,
        policy=env.policy,
        model_hash=env.model_hash,
        variation_id=env.variation_id,
    )

import concurrent.futures
import asyncio
from typing import Coroutine, TypeVar

T = TypeVar("T")

def _run_async_safely(coro: Coroutine[Any, Any, T]) -> T:
    """
    Safely runs a coroutine synchronously, even if an event loop is already running.
    This is necessary for Operators that have synchronous apply() contracts but need
    to make async LLM calls.
    """
    try:
        loop = asyncio.get_running_loop()
    except RuntimeError:
        loop = None
        
    if loop is None:
        return asyncio.run(coro)
    else:
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
            return pool.submit(asyncio.run, coro).result()
