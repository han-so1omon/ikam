from __future__ import annotations

import gzip
import json
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from typing import Any, Mapping, MutableMapping, Optional


@dataclass(frozen=True)
class HugeGraphError(RuntimeError):
    status: int
    body: str

    def __str__(self) -> str:  # pragma: no cover
        return f"HugeGraphError(status={self.status}, body={self.body[:500]!r})"


class HugeGraphClient:
    """Minimal HugeGraph REST client.

    This client intentionally stays small and dependency-free.
    """

    def __init__(
        self,
        *,
        base_url: str,
        graph: str,
        graphspace: str = "DEFAULT",
        timeout_s: float = 10.0,
        headers: Optional[Mapping[str, str]] = None,
    ) -> None:
        self.base_url = base_url.rstrip("/")
        self.graph = graph
        self.graphspace = graphspace
        self.timeout_s = float(timeout_s)
        self._headers = dict(headers or {})
        self._use_graphspace_prefix: Optional[bool] = None

    def list_graphs(self) -> Any:
        return self._request_json("GET", "/graphs")

    def schema_get(self, kind: str, name: str | None = None) -> Any:
        path = f"/schema/{kind}"
        if name is not None:
            path = f"{path}/{urllib.parse.quote(str(name))}"
        return self._request_json("GET", self._graph_scoped(path))

    def schema_create(self, kind: str, payload: Mapping[str, Any]) -> Any:
        return self._request_json("POST", self._graph_scoped(f"/schema/{kind}"), json_body=payload)

    def create_vertex(
        self,
        *,
        label: str,
        properties: Mapping[str, Any],
        vertex_id: str | None = None,
    ) -> Any:
        payload: MutableMapping[str, Any] = {
            "label": label,
            "properties": dict(properties),
        }
        if vertex_id is not None:
            payload["id"] = vertex_id
        return self._request_json("POST", self._graph_scoped("/graph/vertices"), json_body=payload)

    def get_vertex(self, vertex_id: str) -> Any:
        return self._request_json("GET", self._graph_scoped(f"/graph/vertices/{urllib.parse.quote(str(vertex_id))}"))

    def create_edge(
        self,
        *,
        label: str,
        out_v: str,
        in_v: str,
        out_v_label: str,
        in_v_label: str,
        properties: Mapping[str, Any],
    ) -> Any:
        payload = {
            "label": label,
            "outV": out_v,
            "inV": in_v,
            "outVLabel": out_v_label,
            "inVLabel": in_v_label,
            "properties": dict(properties),
        }
        return self._request_json("POST", self._graph_scoped("/graph/edges"), json_body=payload)

    def list_edges(
        self,
        *,
        vertex_id: str,
        direction: str = "BOTH",
        label: str | None = None,
        properties: Mapping[str, Any] | None = None,
        limit: int = 100,
    ) -> Any:
        params: MutableMapping[str, str] = {
            "vertex_id": json.dumps(vertex_id),
            "direction": direction,
            "limit": str(int(limit)),
        }
        if label:
            params["label"] = label
        if properties:
            params["properties"] = json.dumps(dict(properties), separators=(",", ":"), ensure_ascii=False)
        return self._request_json("GET", self._graph_scoped("/graph/edges"), query=params)

    def delete_edge(self, *, edge_id: str, label: str | None = None) -> None:
        q: MutableMapping[str, str] = {}
        if label:
            q["label"] = label
        self._request_json("DELETE", self._graph_scoped(f"/graph/edges/{edge_id}"), query=q)

    def delete_vertex(self, vertex_id: str, label: str | None = None) -> None:
        q: MutableMapping[str, str] = {}
        if label:
            q["label"] = label
        self._request_json("DELETE", self._graph_scoped(f"/graph/vertices/{urllib.parse.quote(str(vertex_id))}"), query=q)

    def gremlin_query(self, gremlin: str, bindings: Optional[Mapping[str, Any]] = None) -> Any:
        payload = {"gremlin": gremlin}
        if bindings:
            payload["bindings"] = dict(bindings)
        return self._request_json("POST", self._graph_scoped("/gremlin"), json_body=payload)

    def _graph_scoped(self, suffix: str) -> str:
        suffix = "/" + suffix.lstrip("/")
        if self._use_graphspace_prefix is None:
            try:
                self._request_json(
                    "GET",
                    f"/graphspaces/{self.graphspace}/graphs/{self.graph}/schema/propertykeys",
                )
                self._use_graphspace_prefix = True
            except Exception:
                self._use_graphspace_prefix = False

        if self._use_graphspace_prefix:
            return f"/graphspaces/{self.graphspace}/graphs/{self.graph}{suffix}"
        return f"/graphs/{self.graph}{suffix}"

    def _request_json(
        self,
        method: str,
        path: str,
        *,
        query: Optional[Mapping[str, str]] = None,
        json_body: Optional[Mapping[str, Any]] = None,
    ) -> Any:
        url = self.base_url + path
        if query:
            url += "?" + urllib.parse.urlencode(query)

        data: Optional[bytes] = None
        headers = {"Accept": "application/json", **self._headers}
        if json_body is not None:
            data = json.dumps(json_body, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
            headers["Content-Type"] = "application/json"

        req = urllib.request.Request(url=url, method=method, data=data, headers=headers)

        try:
            with urllib.request.urlopen(req, timeout=self.timeout_s) as resp:
                body = resp.read()
                encoding = (resp.headers.get("Content-Encoding") or "").lower().strip()
                if encoding == "gzip" or body[:2] == b"\x1f\x8b":
                    body = gzip.decompress(body)
                raw = body.decode("utf-8")
                if raw == "":
                    return {}
                try:
                    return json.loads(raw)
                except json.JSONDecodeError:
                    return {"raw": raw}
        except urllib.error.HTTPError as e:
            body = ""
            try:
                raw_body = e.read()
                encoding = (e.headers.get("Content-Encoding") or "").lower().strip() if e.headers else ""
                if encoding == "gzip" or raw_body[:2] == b"\x1f\x8b":
                    raw_body = gzip.decompress(raw_body)
                body = raw_body.decode("utf-8")
            except Exception:
                body = ""
            raise HugeGraphError(status=int(getattr(e, "code", 0) or 0), body=body) from None
