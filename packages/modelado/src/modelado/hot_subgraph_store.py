import hashlib
import json
from typing import Protocol, TypedDict


class SubgraphRef(TypedDict):
    type: str
    head_fragment_id: str


JsonScalar = str | int | float | bool | None
JsonValue = JsonScalar | list["JsonValue"] | dict[str, "JsonValue"]


class HotSubgraphStore(Protocol):
    def put(self, payload: JsonValue) -> SubgraphRef:
        ...

    def get(self, ref: SubgraphRef) -> JsonValue | None:
        ...


class InMemoryHotSubgraphStore:
    def __init__(self) -> None:
        self._payloads: dict[str, bytes] = {}

    def put(self, payload: JsonValue) -> SubgraphRef:
        canonical = _canonical_payload_bytes(payload)
        digest = hashlib.sha256(canonical).hexdigest()
        head_fragment_id = f"cas://sha256:{digest}"
        self._payloads[head_fragment_id] = canonical
        return {
            "type": "subgraph_ref",
            "head_fragment_id": head_fragment_id,
        }

    def get(self, ref: SubgraphRef) -> JsonValue | None:
        canonical = self._payloads.get(ref["head_fragment_id"])
        if canonical is None:
            return None
        return json.loads(canonical)


def _canonical_payload_bytes(payload: JsonValue) -> bytes:
    return json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode("utf-8")
