#!/usr/bin/env python3
"""E358: ranked and semantic search through `ikam db serve`, against what a
client can do without it (scan the rows and match substrings).

    python3 -I search_bench.py IKAM_BIN WORK_DIR DATA_DIR [FLAGS...]

DATA_DIR comes from `prepare.py` (exp/259-fusion-tune): docs.tsv, the
whole-README vectors doc_whole.f32, queries.tsv and queries.f32. Each
README is stored as a row `t/NAME` (its paragraphs, blank-line separated)
and its vector as `v/NAME` (256 f32, little endian), in 2,000-row batches.
Then, through HTTP on 127.0.0.1:
- quality (nDCG@10, dev and test halves, paired standard error against
  BM25) of: the whole query as a substring, every query word as a
  substring (both key order, what `scan` plus client code gives), BM25
  (`search`), vectors (`vsearch`), and fused (`search&vectors=`, convex at
  three weights and RRF);
- latency of each, the first query of each index (its build), an update
  (1% of the rows rewritten) and a restart.
Prints one JSON line a measurement. Nothing from the data is committed.
"""
import http.client
import json
import math
import os
import random
import shutil
import socket
import struct
import subprocess
import sys
import time
import urllib.parse

import numpy as np

BIN, WORK, DATA = sys.argv[1], sys.argv[2], sys.argv[3]
FLAGS = sys.argv[4:] or ["--live"]
BATCH = 2000
RS = "\x1e"


def out(**kw):
    print(json.dumps(kw), flush=True)


def pack(*parts):
    return b"".join(struct.pack(">I", len(p)) + p for p in parts)


def upsert(key, val, idem):
    return b"\x00" + pack(key.encode(), idem.encode(), val)


def port_free():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


class Server:
    def __init__(self, store, flags):
        self.store, self.flags, self.port, self.proc = store, flags, port_free(), None

    def start(self):
        log = open(os.path.join(WORK, "db.log"), "ab")
        self.proc = subprocess.Popen([BIN, "--store", self.store, "db", "serve", *self.flags,
                                      str(self.port)], stdout=log, stderr=log)
        for _ in range(200):
            try:
                self.conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=600)
                self.req("GET", "/db/")
                return
            except OSError:
                time.sleep(0.05)
        raise SystemExit("server did not start")

    def stop(self):
        self.proc.terminate()
        self.proc.wait()

    def rss_mb(self):
        for line in open(f"/proc/{self.proc.pid}/status"):
            if line.startswith("VmRSS:"):
                return int(line.split()[1]) / 1024

    def req(self, method, path, body=None, ctype=None):
        h = {"Host": f"127.0.0.1:{self.port}"}
        if ctype:
            h["Content-Type"] = ctype
        try:
            self.conn.request(method, path, body=body, headers=h)
            r = self.conn.getresponse()
        except (http.client.RemoteDisconnected, BrokenPipeError, ConnectionResetError):
            # The server closes a connection idle for 2 minutes.
            self.conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=600)
            self.conn.request(method, path, body=body, headers=h)
            r = self.conn.getresponse()
        data = r.read()
        if r.status != 200:
            raise RuntimeError(f"{method} {path[:80]}: {r.status} {data[:200]!r}")
        return data


def unframe(b):
    out, i = [], 0
    while i < len(b):
        n = struct.unpack(">I", b[i:i + 4])[0]
        k = b[i + 4:i + 4 + n].decode()
        i += 4 + n
        n = struct.unpack(">I", b[i:i + 4])[0]
        out.append((k, b[i + 4:i + 4 + n]))
        i += 4 + n
    return out


def ndcg10(ranked, rel):
    dcg = sum(1 / math.log2(i + 2) for i, k in enumerate(ranked[:10]) if k in rel)
    ideal = sum(1 / math.log2(i + 2) for i in range(min(len(rel), 10)))
    return dcg / ideal


def summary(scores, base=None):
    """Mean by task and split, and the paired standard error against base."""
    res = {}
    for task in ("known", "topic"):
        for split in ("dev", "test"):
            ids = [i for i, q in enumerate(QUERIES) if q[0] == task and q[2] == split]
            m = float(np.mean([scores[i] for i in ids]))
            cell = {"n": len(ids), "ndcg10": round(m, 4)}
            if base is not None:
                d = np.array([scores[i] - base[i] for i in ids])
                cell["diff"] = round(float(d.mean()), 4)
                cell["se"] = round(float(d.std(ddof=1) / math.sqrt(len(d))), 4)
            res[f"{task}/{split}"] = cell
    return res


def pct(xs, p):
    xs = sorted(xs)
    return round(xs[min(len(xs) - 1, int(p * len(xs)))] * 1000, 2)


# Data.
docs = []
for line in open(os.path.join(DATA, "docs.tsv"), encoding="utf-8"):
    key, paras = line.rstrip("\n").split("\t", 1)
    docs.append((key[2:], paras.replace(RS, "\n\n")))
dim = 256
vecs = np.fromfile(os.path.join(DATA, "doc_whole.f32"), dtype="<f4").reshape(len(docs), dim)
QUERIES = []
for line in open(os.path.join(DATA, "queries.tsv"), encoding="utf-8"):
    task, qid, split, rel, text = line.rstrip("\n").split("\t", 4)
    QUERIES.append((task, qid, split, {"t/" + k[2:] for k in rel.split(",")}, text))
qvecs = np.fromfile(os.path.join(DATA, "queries.f32"), dtype="<f4").reshape(len(QUERIES), dim)
out(docs=len(docs), queries=len(QUERIES), dim=dim, flags=FLAGS,
    text_mb=round(sum(len(t.encode()) for _, t in docs) / 1e6, 1))

shutil.rmtree(WORK, ignore_errors=True)
os.makedirs(WORK)
store = os.path.join(WORK, "store")
srv = Server(store, FLAGS)
srv.start()

# Load.
t = time.perf_counter()
for s in range(0, len(docs), BATCH):
    ev = []
    for i in range(s, min(s + BATCH, len(docs))):
        name, text = docs[i]
        ev.append(upsert("t/" + name, text.encode(), f"t{i}"))
        ev.append(upsert("v/" + name, vecs[i].astype("<f4").tobytes(), f"v{i}"))
    srv.req("POST", "/db/r/append", b"".join(ev), "application/x-ikam-events")


def du(p):
    return sum(os.path.getsize(os.path.join(d, f)) for d, _, fs in os.walk(p) for f in fs)


out(step="load", s=round(time.perf_counter() - t, 1), store_mb=round(du(store) / 1e6, 1),
    rss_mb=srv.rss_mb())


def search(q, k=10):
    return "/db/r/search?" + urllib.parse.urlencode({"q": q, "prefix": "t/", "k": k})


def keys(body):
    return [h["key"] for h in json.loads(body)]


def timed(fn, n=None):
    """Run fn over the queries; returns (scores, seconds per query)."""
    scores, secs = [], []
    for i, q in enumerate(QUERIES[:n] if n else QUERIES):
        t = time.perf_counter()
        ranked = fn(i, q)
        secs.append(time.perf_counter() - t)
        scores.append(ndcg10(ranked, q[3]))
    return scores, secs


def vec(i):
    return qvecs[i].astype("<f4").tobytes()


# First queries: each builds its index.
before = du(store)
t = time.perf_counter()
srv.req("GET", search(QUERIES[0][4]))
bm25_build = time.perf_counter() - t
t = time.perf_counter()
srv.req("POST", "/db/r/vsearch?prefix=v/&k=10", vec(0))
vec_build = time.perf_counter() - t
out(step="first-query", bm25_s=round(bm25_build, 2), vectors_s=round(vec_build, 2),
    bm25_view_written_mb=round((du(store) - before) / 1e6, 1), rss_mb=srv.rss_mb())

# Substring baselines: what `scan` and client code give, in key order.
t = time.perf_counter()
rows = [(k, v.decode("utf-8", "replace").lower()) for k, v in
        unframe(srv.req("GET", "/db/r/scan?prefix=t/"))]
scan_s = time.perf_counter() - t


def sub_whole(i, q):
    s = q[4].lower()
    return [k for k, v in rows if s in v][:10]


def sub_words(i, q):
    ws = q[4].lower().split()
    return [k for k, v in rows if all(w in v for w in ws)][:10]


runs = {}
for name, fn in (("substring-whole", sub_whole), ("substring-words", sub_words)):
    sc, secs = timed(fn)
    runs[name] = sc
    out(ranking=name, latency_ms={"scan": round(scan_s * 1000, 1),
                                  "match_p50": pct(secs, 0.5)}, **summary(sc))

bm25, secs = timed(lambda i, q: keys(srv.req("GET", search(q[4]))))
runs["bm25"] = bm25
out(ranking="bm25", latency_ms={"p50": pct(secs, 0.5), "p95": pct(secs, 0.95)}, **summary(bm25))
for name in ("substring-whole", "substring-words"):
    out(compare=name, vs="bm25", **summary(runs[name], bm25))

# vsearch answers v/ keys: map them to t/ for judging.
sc, secs = timed(lambda i, q: ["t/" + k[2:] for k in
                               keys(srv.req("POST", "/db/r/vsearch?prefix=v/&k=10", vec(i)))])
out(ranking="vectors", latency_ms={"p50": pct(secs, 0.5), "p95": pct(secs, 0.95)},
    **summary(sc, bm25))
for fuse in ("alpha=0.5", "alpha=0.6", "alpha=0.7", "fuse=rrf"):
    def fused(i, q):
        return keys(srv.req("POST", search(q[4]) + "&vectors=v/&" + fuse, vec(i)))
    sc, secs = timed(fused)
    out(ranking="fused " + fuse, latency_ms={"p50": pct(secs, 0.5), "p95": pct(secs, 0.95)},
        **summary(sc, bm25))

# An update: 1% of the READMEs given other READMEs' text and vectors.
rnd = random.Random(358)
n = max(1, len(docs) // 100)
ev = []
for j, i in enumerate(rnd.sample(range(len(docs)), n)):
    o = rnd.randrange(len(docs))
    ev.append(upsert("t/" + docs[i][0], docs[o][1].encode(), f"u{j}"))
    ev.append(upsert("v/" + docs[i][0], vecs[o].astype("<f4").tobytes(), f"uv{j}"))
t = time.perf_counter()
srv.req("POST", "/db/r/append", b"".join(ev), "application/x-ikam-events")
append_s = time.perf_counter() - t
t = time.perf_counter()
srv.req("GET", search(QUERIES[1][4]))
bm25_upd = time.perf_counter() - t
t = time.perf_counter()
srv.req("GET", search(QUERIES[2][4]))
bm25_next = time.perf_counter() - t
t = time.perf_counter()
srv.req("POST", "/db/r/vsearch?prefix=v/&k=10", vec(1))
vec_upd = time.perf_counter() - t
_, secs = timed(lambda i, q: keys(srv.req("GET", search(q[4]))), 300)
out(step="update-1pct", rows=n, append_s=round(append_s, 3), bm25_first_s=round(bm25_upd, 3),
    bm25_next_ms=round(bm25_next * 1000, 1), vectors_first_s=round(vec_upd, 3),
    bm25_p50_ms_after=pct(secs, 0.5), rss_mb=srv.rss_mb())

# A restart: the indexes are built again on the first queries.
srv.stop()
srv.start()
t = time.perf_counter()
srv.req("GET", search(QUERIES[0][4]))
r_bm25 = time.perf_counter() - t
t = time.perf_counter()
srv.req("POST", "/db/r/vsearch?prefix=v/&k=10", vec(0))
r_vec = time.perf_counter() - t
out(step="restart", bm25_first_s=round(r_bm25, 2), vectors_first_s=round(r_vec, 2),
    store_mb=round(du(store) / 1e6, 1), rss_mb=srv.rss_mb())
srv.stop()
