"""E388: does `ikam mcp`'s `rank` tool rank whole files as `db serve`'s
search route ranks the same texts as rows, keyword-only and fused with
vectors, and how fast does it answer?

    python3 -I rank_bench.py IKAM_BIN PYPI_JSONL WORK [N_QUERIES]
    python3 -I rank_bench.py IKAM_BIN --time STORE WORK

PYPI_JSONL is the output of exp/259-fusion-tune's examples/real_text/fetch.py
(PyPI project pages: name, summary, README). OSI-licensed projects with a
README of at least two paragraphs are kept, as prepare.py keeps them. Each
README becomes the file `t/SLUG` of a store (`ikam ingest`) and the row
`t/SLUG` of a `db serve` database, so file paths and row keys are the same
strings. Vectors: a hashed bag of words (256 dimensions, unit length), the
same numbers on both sides; a stand-in for a real embedder, enough to test
that fusion matches, not to judge ranking quality. Queries: projects'
summaries (known items).

The comparison: the top 10 keys and scores of `rank` (keywords) against
`GET search`, and of `rank` with the query vector against `POST search
...&vectors=v/` (convex, alpha 0.5). Times: client-side per call through
stdio, after one warm-up call.

`--time STORE`: `rank` (keywords) on an existing store, 20 questions 5 times
each, with `--rank-files`; for E378's eight-copy store. Nothing from the
data is committed; WORK is scratch. 127.0.0.1 only.
"""
import base64, hashlib, http.client, json, os, random, re, shutil, socket, struct
import subprocess, sys, time

import numpy as np

binary = sys.argv[1]
PW = "e388-test"
DIM = 256
LINE = re.compile(r"^(\d+)\. (.*) \(score (-?[0-9.]+)\)$")


def terms(text):
    return [t.lower() for t in re.split(r"[^\w]|_", text) if t]


def hvec(text):
    v = np.zeros(DIM, dtype=np.float64)
    for t in terms(text):
        h = hashlib.blake2b(t.encode(), digest_size=8).digest()
        n = int.from_bytes(h, "little")
        v[n % DIM] += 1.0 if (n >> 32) & 1 else -1.0
    norm = np.linalg.norm(v)
    return (v / norm if norm > 0 else v).astype(np.float32)


class Mcp:
    def __init__(self, store, flags):
        t = time.time()
        self.p = subprocess.Popen([binary, "--store", store, "mcp", *flags], stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        self.n = 0
        self.rpc("initialize", {"protocolVersion": "2025-06-18"})
        self.ready = time.time() - t

    def rpc(self, method, params):
        self.n += 1
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.n, "method": method,
                                       "params": params}) + "\n")
        self.p.stdin.flush()
        return json.loads(self.p.stdout.readline())

    def rank(self, args):
        t = time.perf_counter()
        r = self.rpc("tools/call", {"name": "rank", "arguments": args})
        dt = time.perf_counter() - t
        text = r["result"]["content"][0]["text"]
        out = []
        for line in text.splitlines():
            m = LINE.match(line)
            if m:
                out.append((m.group(2), float(m.group(3))))
        return out, dt

    def rss(self):
        for line in open("/proc/%d/status" % self.p.pid):
            if line.startswith("VmRSS"):
                return round(int(line.split()[1]) / 1024)

    def close(self):
        self.p.stdin.close(); self.p.wait(60)


def pack(*parts):
    return b"".join(struct.pack(">I", len(p)) + p for p in parts)


class Db:
    def __init__(self, store):
        s = socket.socket(); s.bind(("127.0.0.1", 0)); self.port = s.getsockname()[1]; s.close()
        line = subprocess.run([binary, "passwd", "u"], input=PW + "\n", capture_output=True,
                              text=True, check=True).stdout.strip()
        self.hdr = {"Authorization": "Basic " + base64.b64encode(("u:" + PW).encode()).decode()}
        self.p = subprocess.Popen([binary, "--store", store, "db", "serve", str(self.port)],
                                  env=dict(os.environ, IKAM_AUTH=line.split("=", 1)[1]),
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(600):
            try:
                self.c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=600)
                self.call("GET", "/db/r/head")
                return
            except OSError:
                time.sleep(0.1)
        raise RuntimeError("db serve did not start")

    def call(self, method, path, body=None, ctype="application/octet-stream"):
        h = dict(self.hdr)
        if body is not None:
            h["Content-Type"] = ctype
        self.c.request(method, path, body=body, headers=h)
        r = self.c.getresponse()
        data = r.read()
        if r.status != 200:
            raise RuntimeError("%s %s: %d %s" % (method, path, r.status, data[:200]))
        return data

    def load(self, rows):
        for i in range(0, len(rows), 500):
            out = [b"\x00" + pack(k.encode(), b"e%d" % (i + j), v)
                   for j, (k, v) in enumerate(rows[i:i + 500])]
            self.call("POST", "/db/r/append", b"".join(out), "application/x-ikam-events")

    def search(self, q, vec=None):
        qs = "q=" + urllib_quote(q) + "&prefix=t/&k=10"
        if vec is None:
            b = self.call("GET", "/db/r/search?" + qs)
        else:
            b = self.call("POST", "/db/r/search?" + qs + "&vectors=v/", vec.tobytes())
        return [(h["key"], h["score"]) for h in json.loads(b)]

    def close(self):
        self.c.close(); self.p.terminate(); self.p.wait(60)


def urllib_quote(s):
    from urllib.parse import quote
    return quote(s, safe="")


def ms(xs):
    xs = sorted(xs)
    return {"p50_ms": round(xs[len(xs) // 2] * 1e3, 2),
            "p95_ms": round(xs[max(0, int(len(xs) * 0.95) - 1)] * 1e3, 2)}


def same(a, b):
    return [k for k, _ in a] == [k for k, _ in b] and all(
        abs(x - y) <= 1e-5 * max(1.0, abs(y)) for (_, x), (_, y) in zip(a, b))


QUESTIONS = [
    "read csv with a dtype per column", "groupby aggregate several functions",
    "merge two frames on the index", "datetime with a timezone", "parquet engine",
    "rolling window mean", "fill missing values", "copy on write", "string accessor",
    "plotting backend", "click option with a default", "http session retries",
    "quarterly revenue", "slide title", "table of contents", "deprecation warning",
    "categorical dtype ordered", "sparse array", "read excel sheet name", "json normalize",
]


def time_only(store, work):
    copy = os.path.join(work, "time.ikam")
    shutil.rmtree(copy, ignore_errors=True); shutil.copytree(store, copy)
    m = Mcp(copy, ["--rank-files"])
    m.rank({"query": "warm up", "passages": 0})
    times = []
    for _ in range(5):
        for q in QUESTIONS:
            times.append(m.rank({"query": q, "k": 10, "passages": 0})[1])
    with_passages = [m.rank({"query": q, "k": 10})[1] for q in QUESTIONS]
    out = {"store": os.path.basename(store.rstrip("/")), "ready_s": round(m.ready, 1),
           "rss_mb": m.rss(), "rank": ms(times), "rank_with_passages": ms(with_passages)}
    m.close()
    shutil.rmtree(copy, ignore_errors=True)
    print(json.dumps(out))


def main():
    if sys.argv[2] == "--time":
        return time_only(sys.argv[3], sys.argv[4])
    src, work = sys.argv[2], sys.argv[3]
    nq = int(sys.argv[4]) if len(sys.argv) > 4 else 300
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(os.path.join(work, "tree", "t"))
    docs, seen = [], set()
    for line in open(src):
        p = json.loads(line)
        slug = re.sub(r"[^a-z0-9]+", "-", p["name"].lower()).strip("-")
        paras = [x for x in re.split(r"\n\s*\n", p["description"]) if x.strip()]
        if slug in seen or len(paras) < 2 or not slug:
            continue
        seen.add(slug)
        docs.append((slug, p["summary"], p["description"]))
    docs.sort()
    vecs = np.stack([hvec(d[2]) for d in docs])
    for slug, _, text in docs:
        with open(os.path.join(work, "tree", "t", slug), "w", encoding="utf-8") as f:
            f.write(text)
    with open(os.path.join(work, "vecs.paths"), "w") as f:
        f.write("".join("t/%s\n" % d[0] for d in docs))
    vecs.tofile(os.path.join(work, "vecs.f32"))
    files = os.path.join(work, "files.ikam")
    subprocess.run([binary, "--store", files, "ingest", os.path.join(work, "tree")],
                   check=True, capture_output=True)
    rows = os.path.join(work, "rows.ikam")
    db = Db(rows)
    db.load([("t/" + s, open(os.path.join(work, "tree", "t", s), "rb").read()) for s, _, _ in docs]
            + [("v/" + d[0], vecs[i].tobytes()) for i, d in enumerate(docs)])
    m = Mcp(files, ["--rank-files", "--file-vectors", os.path.join(work, "vecs")])
    qs = random.Random(388).sample(docs, min(nq, len(docs)))
    res = {"docs": len(docs), "queries": len(qs), "mcp_ready_s": round(m.ready, 1),
           "mcp_rss_mb": m.rss()}
    m.rank({"query": "warm up", "passages": 0}); db.search("warm up")
    for mode in ("keywords", "fused"):
        agree, t_mcp, t_db, diffs = 0, [], [], []
        for slug, summary, _ in qs:
            qv = hvec(summary) if mode == "fused" else None
            args = {"query": summary, "k": 10, "passages": 0}
            if qv is not None:
                args["vector"] = [float(x) for x in qv]
            got, dt = m.rank(args)
            t_mcp.append(dt)
            t = time.perf_counter()
            want = db.search(summary, qv)
            t_db.append(time.perf_counter() - t)
            if same(got, want):
                agree += 1
            elif len(diffs) < 3:
                diffs.append({"q": summary, "mcp": got[:3], "db": want[:3]})
        res[mode] = {"same_top10": "%d/%d" % (agree, len(qs)), "mcp": ms(t_mcp), "db_serve": ms(t_db),
                     "first_differences": diffs}
    with_passages = [m.rank({"query": s, "k": 10})[1] for _, s, _ in qs[:50]]
    res["mcp_with_passages"] = ms(with_passages)
    m.close(); db.close()
    print(json.dumps(res, indent=1))


main()
