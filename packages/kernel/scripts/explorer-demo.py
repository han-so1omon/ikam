"""E440: helpers for explorer-demo.sh, run as `python3 -I explorer-demo.py CMD ...`.

  mcp BIN STORE OUT         a short MCP session with `ikam mcp`, written as Markdown
  db BIN DIR PORT OUT       a short `ikam db serve` session (IKAM_AUTH and DEMO_USER,
                            DEMO_PW in the environment), written as Markdown
  wiki DUMP PAGES OUT       the dump's head and its first PAGES pages, one file each,
                            exactly as their bytes stand; prints the slice's sha256
  join DIR                  the sha256 of head + pages in order, as `wiki` wrote them

Only the standard library; nothing leaves 127.0.0.1.
"""

import base64
import hashlib
import json
import os
import re
import struct
import subprocess
import sys
import time
import urllib.error
import urllib.request


def mcp(binary, store, out):
    calls = [
        ("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                        "clientInfo": {"name": "explorer-demo", "version": "1"}}),
        ("tools/list", {}),
        ("tools/call", {"name": "list", "arguments": {"path": ""}}),
        ("tools/call", {"name": "search", "arguments": {"query": "scoped share", "limit": 5}}),
        ("tools/call", {"name": "rank", "arguments": {"query": "how are projects kept apart", "limit": 3}}),
        ("tools/call", {"name": "history", "arguments": {"limit": 3}}),
    ]
    lines = []
    for i, (method, params) in enumerate(calls, 1):
        lines.append(json.dumps({"jsonrpc": "2.0", "id": i, "method": method, "params": params}))
        if method == "initialize":
            lines.append(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}))
    p = subprocess.run([binary, "--store", store, "mcp"], input="\n".join(lines) + "\n",
                       capture_output=True, text=True, timeout=300)
    answers = {}
    for line in p.stdout.splitlines():
        try:
            a = json.loads(line)
        except ValueError:
            continue
        if "id" in a:
            answers[a["id"]] = a
    doc = ["# A session with `ikam mcp`", "",
           "Each step is one JSON-RPC line sent to `ikam --store STORE mcp` on stdin, and its answer "
           "(long answers cut). An agent such as Claude Code connects with "
           "`claude mcp add ikam -- ikam --store STORE mcp`.", ""]
    for i, (method, params) in enumerate(calls, 1):
        a = answers.get(i, {"error": "no answer"})
        text = json.dumps(a.get("result", a.get("error")), indent=1)
        if len(text) > 3000:
            text = text[:3000] + "\n... (cut)"
        doc += [f"## {i}. {method} {params.get('name', '')}".rstrip(), "", "```json",
                json.dumps(params), "```", "", "```json", text, "```", ""]
    with open(out, "w") as f:
        f.write("\n".join(doc))
    print(f"mcp: {len(answers)} of {len(calls)} calls answered")


def frames(body):
    pairs, i = [], 0
    while i + 4 <= len(body):
        n = struct.unpack(">I", body[i:i + 4])[0]
        k = body[i + 4:i + 4 + n]
        i += 4 + n
        m = struct.unpack(">I", body[i:i + 4])[0]
        pairs.append((k.decode(), body[i + 4:i + 4 + m].decode()))
        i += 4 + m
    return pairs


def db(binary, store, port, out):
    auth = "Basic " + base64.b64encode(
        f"{os.environ['DEMO_USER']}:{os.environ['DEMO_PW']}".encode()).decode()
    base = f"http://127.0.0.1:{port}/db/notes/"
    log = open(os.path.join(os.path.dirname(out), "db-serve.log"), "w")
    srv = subprocess.Popen([binary, "--store", store, "db", "serve", "--live", port],
                           stdout=log, stderr=log)
    doc = ["# A session with `ikam db serve`", "",
           "The database mode: each database is a log of keyed events, committed before it is "
           "answered, so every earlier version stays readable (`at=COMMIT`). Calls over HTTP on "
           "127.0.0.1, as a program makes them.", ""]

    def call(method, route, body=None, kind=None):
        r = urllib.request.Request(base + route, data=body, method=method)
        r.add_header("Authorization", auth)
        r.add_header("X-Ikam", "1")
        if kind:
            r.add_header("Content-Type", kind)
        try:
            with urllib.request.urlopen(r, timeout=30) as a:
                return a.status, a.read()
        except urllib.error.HTTPError as e:
            return e.code, e.read()

    def events(*ups):
        b = b""
        for k, v in ups:
            b += b"\x00"
            for part in (k, "demo-" + k + "-" + v, v):
                part = part.encode()
                b += struct.pack(">I", len(part)) + part
        return b

    try:
        for _ in range(100):
            try:
                if call("GET", "head")[0] in (200, 404):
                    break
            except OSError:
                time.sleep(0.1)
        kv = "application/x-ikam-events"
        steps = [
            ("POST", "append", events(("city/lisbon", "Portugal"), ("city/porto", "Portugal"),
                                      ("city/oslo", "Norway")), kv, "append three records"),
            ("GET", "head", None, None, "the log's head"),
            ("GET", "scan?prefix=city/&format=json", None, None, "every city, in key order"),
            ("POST", "append", events(("city/oslo", "Norway (capital)")), kv, "change one record"),
            ("GET", "get?key=city/oslo", None, None, "its value now"),
            ("GET", "count?prefix=city/", None, None, "count without reading values"),
        ]
        first = None
        for method, route, body, kind, what in steps:
            st, got = call(method, route, body, kind)
            if route == "append" and first is None:
                first = json.loads(got).get("commit")
            doc += [f"## {what}", "", f"`{method} /db/notes/{route}` -> {st}", "", "```",
                    got.decode(errors="replace")[:2000], "```", ""]
        if first:
            st, got = call("GET", f"get?key=city/oslo&at={first}")
            doc += ["## the same record at the first commit", "",
                    f"`GET /db/notes/get?key=city/oslo&at={first[:12]}...` -> {st}", "", "```",
                    got.decode(errors="replace"), "```", ""]
    finally:
        srv.terminate()
        srv.wait(timeout=30)
    with open(out, "w") as f:
        f.write("\n".join(doc))
    print(f"db: {sum(1 for d in doc if d.startswith('## '))} steps written")


def safe(title):
    t = re.sub(r"[^A-Za-z0-9 ._()-]+", "_", title).strip(" ._") or "page"
    return t[:80]


def wiki(dump, pages, out):
    pages = int(pages)
    with open(dump, "rb") as f:
        data = f.read(64 << 20)
    starts = [m.start() for m in re.finditer(rb"<page>", data)]
    if len(starts) <= pages:
        sys.exit(f"wiki: {dump} has fewer than {pages + 1} pages in its first 64 MB")
    end = starts[pages]
    os.makedirs(os.path.join(out, "pages"))
    with open(os.path.join(out, "head.xml"), "wb") as f:
        f.write(data[:starts[0]])
    order = []
    for i in range(pages):
        chunk = data[starts[i]:starts[i + 1]]
        m = re.search(rb"<title>(.*?)</title>", chunk)
        name = f"{i:05d} {safe(m.group(1).decode(errors='replace') if m else 'page')}.xml"
        order.append(name)
        with open(os.path.join(out, "pages", name), "wb") as f:
            f.write(chunk)
    links = sum(len(re.findall(rb"\[\[", data[s:e])) for s, e in zip(starts, starts[1:pages + 1]))
    print(hashlib.sha256(data[:end]).hexdigest(), end, links)


def join(d):
    h = hashlib.sha256()
    with open(os.path.join(d, "head.xml"), "rb") as f:
        h.update(f.read())
    for name in sorted(os.listdir(os.path.join(d, "pages"))):
        with open(os.path.join(d, "pages", name), "rb") as f:
            h.update(f.read())
    print(h.hexdigest())


if __name__ == "__main__":
    cmd, args = sys.argv[1], sys.argv[2:]
    {"mcp": mcp, "db": db, "wiki": wiki, "join": join}[cmd](*args)
