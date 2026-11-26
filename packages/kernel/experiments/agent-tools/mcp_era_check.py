#!/usr/bin/env python3
"""E406: does `ikam mcp` answer 2026-07-28 clients with the same content it
gives handshake-era clients, in one process, in either order?

    python3 mcp_era_check.py IKAM_BIN WORK_DIR CORPUS_DIR REQUESTS_IN

REQUESTS_IN is the client side of an `mcp_check.py` run (E359), one JSON-RPC
message a line, as a tap between the SDK and `ikam mcp` recorded it. Four
fresh copies of one store replay it:

- `legacy`: the requests as recorded (initialize, then tools/list and calls);
- `modern`: each request in the 2026-07-28 envelope (`_meta` with the
  version and capabilities, no initialize, a `server/discover` first);
- `mixed`: odd requests legacy, even ones modern;
- `mixed2`: the other way round.

Every answer is compared with the legacy one after dropping the fields only
2026-07-28 results carry (`resultType`, `ttlMs`, `cacheScope`, `_meta`) and
naming commits by their order of appearance (commit ids hash a time). Each
2026-07-28 answer is also checked against that revision's wire schema from
the official Python SDK (`mcp_types`). Then the official SDK's own client
(`mcp.client.client.Client`, mode `auto`, which probes `server/discover` and
falls back to `initialize`) runs an agent's path, and the error cases are
sent by hand. Prints one JSON line. Local files only.
"""
import asyncio
import copy
import json
import os
import re
import shutil
import subprocess
import sys

from mcp import StdioServerParameters
from mcp.client.client import Client
from mcp_types import _v2026_07_28 as wire

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from mcp_check import build_store  # noqa: E402  (E359's corpus and store)

VERSION = "2026-07-28"
META = {
    "io.modelcontextprotocol/protocolVersion": VERSION,
    "io.modelcontextprotocol/clientInfo": {"name": "e406", "version": "0"},
    "io.modelcontextprotocol/clientCapabilities": {},
}
MODERN_ONLY = ("resultType", "ttlMs", "cacheScope", "_meta")
HEX = re.compile(r"(?<![0-9a-f])[0-9a-f]{64}(?![0-9a-f])")
SCHEMA = {
    "server/discover": wire.DiscoverResult,
    "tools/list": wire.ListToolsResult,
    "tools/call": wire.CallToolResult,
}


class Server:
    def __init__(self, ikam, store):
        self.p = subprocess.Popen(
            [ikam, "--store", store, "mcp"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
        )
        self.ids = {}  # recorded commit id -> this store's

    def send(self, msg):
        line = HEX.sub(lambda m: self.ids.get(m.group(0), m.group(0)), json.dumps(msg))
        self.p.stdin.write(line + "\n")
        self.p.stdin.flush()
        if "id" not in msg:
            return None
        return self.p.stdout.readline()

    def close(self):
        self.p.stdin.close()
        self.p.wait(timeout=120)


def modern(msg):
    m = copy.deepcopy(msg)
    m.setdefault("params", {})["_meta"] = dict(META)
    return m


def strip(answer):
    a = json.loads(answer)
    for k in MODERN_ONLY:
        a.get("result", {}).pop(k, None)
    return a


def norm(text, names):
    return HEX.sub(lambda m: "C%d" % names.setdefault(m.group(0), len(names) + 1), text)


def replay(ikam, store, recorded, era_of):
    """Send `recorded` to a fresh server; `era_of(i)` picks each request's era."""
    s = Server(ikam, store)
    out, bad_schema, modern_n = [], [], 0
    if era_of(0) == "modern":
        raw = s.send({"jsonrpc": "2.0", "id": "d", "method": "server/discover", "params": {"_meta": META}})
        bad_schema += check_schema("server/discover", raw)
    i = 0
    for msg, rec_answer in recorded:
        era = era_of(i)
        method = msg.get("method")
        if era == "modern" and method in ("initialize", "notifications/initialized"):
            continue  # this era has no handshake
        i += 1
        raw = s.send(modern(msg) if era == "modern" else msg)
        if raw is None:
            continue
        if era == "modern":
            modern_n += 1
            bad_schema += check_schema(method, raw)
        # Learn this store's commit ids from the answer, in order, so later
        # requests that name a recorded commit name this store's.
        for a, b in zip(HEX.findall(rec_answer), HEX.findall(raw)):
            s.ids.setdefault(a, b)
        out.append((msg.get("id"), strip(raw)))
    s.close()
    return out, bad_schema, modern_n


def check_schema(method, raw):
    a = json.loads(raw)
    if "result" not in a:
        return [(method, "no result")]
    try:
        SCHEMA[method].model_validate(a["result"])
        assert a["result"]["resultType"] == "complete"
    except Exception as e:  # noqa: BLE001
        return [(method, str(e)[:200])]
    return []


async def sdk_client(ikam, store):
    """The official SDK client's own path, as an agent would use it."""
    p = StdioServerParameters(command=ikam, args=["--store", store, "mcp"])
    res = {}
    for mode in ("auto", "legacy"):
        async with Client(p, mode=mode) as c:
            r = {"version": c.session.protocol_version}
            tools = (await c.list_tools()).tools
            r["tools"] = len(tools)
            hit = await c.call_tool("search", {"query": "Configuration"})
            r["search_lines"] = hit.content[0].text.count("\n")
            await c.call_tool("write", {"path": f"era-{mode}.txt", "content": f"written by {mode}\n"})
            done = await c.call_tool("commit", {"message": f"E406 {mode}"})
            r["committed"] = done.content[0].text.startswith("committed")
            back = await c.call_tool("read", {"path": f"era-{mode}.txt"})
            r["read_back"] = f"written by {mode}" in back.content[0].text
            bad = await c.call_tool("read", {"path": "no/such/file"})
            r["bad_path_is_error"] = bad.is_error
            res[mode] = r
    return res


def errors(ikam, store):
    s = Server(ikam, store)
    out = {}

    def code(msg):
        a = json.loads(s.send(msg))
        return a.get("error", {}).get("code"), a.get("error", {}).get("data")

    meta = dict(META, **{"io.modelcontextprotocol/protocolVersion": "2099-01-01"})
    c, d = code({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {"_meta": meta}})
    out["unknown_version"] = c
    out["unknown_version_data_ok"] = d is not None and d["requested"] == "2099-01-01" and VERSION in d["supported"]
    wire.UnsupportedProtocolVersionError.model_validate(
        {"jsonrpc": "2.0", "id": 1, "error": {"code": c, "message": "x", "data": d}}
    )
    meta = {"io.modelcontextprotocol/protocolVersion": VERSION}
    out["missing_capabilities"] = code({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {"_meta": meta}})[0]
    out["discover_without_version"] = code({"jsonrpc": "2.0", "id": 3, "method": "server/discover", "params": {}})[0]
    out["ping_modern"] = code({"jsonrpc": "2.0", "id": 4, "method": "ping", "params": {"_meta": META}})[0]
    out["initialize_modern_envelope"] = code(
        {"jsonrpc": "2.0", "id": 5, "method": "initialize", "params": {"_meta": META, "protocolVersion": "2025-11-25"}}
    )[0]
    # A handshake after modern requests, in the same process, still works.
    a = json.loads(s.send({"jsonrpc": "2.0", "id": 6, "method": "initialize", "params": {"protocolVersion": "2025-11-25"}}))
    out["initialize_after_modern"] = a["result"]["protocolVersion"]
    # A handshake-era call with its own _meta (a progress token) is unchanged.
    a = json.loads(s.send({"jsonrpc": "2.0", "id": 7, "method": "tools/list", "params": {"_meta": {"progressToken": 1}}}))
    out["legacy_meta_untouched"] = "resultType" not in a["result"]
    s.close()
    return out


def main():
    ikam, work, corpus, requests_in = sys.argv[1:5]
    os.makedirs(work, exist_ok=True)
    store, _, files, size, _ = build_store(ikam, work, corpus, 1, False)
    lines = [json.loads(l) for l in open(requests_in)]
    # The recorded answers, for learning commit ids: from the tap's .out.
    answers = open(requests_in[: -len(".in")] + ".out").read().splitlines()
    it = iter(answers)
    recorded = [(m, next(it) if "id" in m else "") for m in lines]
    eras = {
        "legacy": lambda i: "legacy",
        "modern": lambda i: "modern",
        "mixed": lambda i: "legacy" if i % 2 else "modern",
        "mixed2": lambda i: "modern" if i % 2 else "legacy",
    }
    runs = {}
    for name, era in eras.items():
        st = os.path.join(work, f"store-{name}")
        shutil.rmtree(st, ignore_errors=True)
        shutil.copytree(store, st)
        runs[name] = replay(ikam, st, recorded, era)
    res = {"files": files, "corpus_bytes": size, "requests": len(lines)}
    # initialize has no counterpart in the 2026-07-28 era: compare the rest
    # by request id, each side's commits named in its own order.
    base = [(rid, a) for rid, a in runs["legacy"][0] if not a.get("result", {}).get("protocolVersion")]
    for name, (out, bad_schema, modern_n) in runs.items():
        got = dict(out)
        nb, ng, same, differ = {}, {}, 0, []
        for rid, a in base:
            if rid not in got:
                differ.append(rid)
                continue
            x = norm(json.dumps(a.get("result", a.get("error"))), nb)
            y = norm(json.dumps(got[rid].get("result", got[rid].get("error"))), ng)
            if x == y:
                same += 1
            else:
                differ.append(rid)
        res[name] = {
            "answers": len(out),
            "modern": modern_n,
            "compared": len(base),
            "equal_to_legacy": same,
            "differ": differ[:5],
            "schema_errors": bad_schema[:3],
        }
    st = os.path.join(work, "store-sdk")
    shutil.rmtree(st, ignore_errors=True)
    shutil.copytree(store, st)
    res["sdk_client"] = asyncio.run(sdk_client(ikam, st))
    res["errors"] = errors(ikam, st)
    print(json.dumps(res))


if __name__ == "__main__":
    main()
