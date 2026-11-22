#!/usr/bin/env python3
"""E369: the first BM25 query after a restart of `ikam db serve`, with and
without `--keep-search`.

    python3 -I restart_bench.py IKAM_BIN WORK_DIR [ROWS] [FLAGS...]

Makes ROWS (default 11,000) rows `t/N` of generated text (about 4.4 KB
each: 950 words drawn from a 30,000-word vocabulary with Zipf weights, so
about 49 MB in all, near the size of E358's README table), in 2,000-row
batches, on a fresh store. Then, for each step, the time of one query (the same query
text each time) and the store's size:
- the first query (the view is built);
- a restart with no change;
- a restart after 1% of the rows are rewritten (by a server that searched
  nothing);
- a second restart with no change.
Every server keeps the same flags. Prints one JSON line a measurement.
"""
import http.client
import json
import os
import random
import shutil
import socket
import struct
import subprocess
import sys
import time

BIN, WORK = sys.argv[1], sys.argv[2]
ROWS = int(sys.argv[3]) if len(sys.argv) > 3 else 11000
FLAGS = sys.argv[4:] or ["--live"]
BATCH = 2000
WORDS = 950
Q = "/db/r/search?q=w17+w402+w2210&prefix=t/&k=10"


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


def du(p):
    return sum(os.path.getsize(os.path.join(d, f)) for d, _, fs in os.walk(p) for f in fs)


class Server:
    def __init__(self, store):
        self.store, self.port = store, port_free()
        log = open(os.path.join(WORK, "db.log"), "ab")
        self.proc = subprocess.Popen([BIN, "--store", store, "db", "serve", *FLAGS,
                                      str(self.port)], stdout=log, stderr=log)
        for _ in range(400):
            try:
                self.conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=900)
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
                return round(int(line.split()[1]) / 1024)

    def req(self, method, path, body=None, ctype=None):
        h = {"Host": f"127.0.0.1:{self.port}"}
        if ctype:
            h["Content-Type"] = ctype
        self.conn.request(method, path, body=body, headers=h)
        r = self.conn.getresponse()
        data = r.read()
        if r.status != 200:
            raise RuntimeError(f"{method} {path[:80]}: {r.status} {data[:200]!r}")
        return data


rng = random.Random(369)
vocab = [f"w{i}" for i in range(30000)]
weights = [1 / (i + 1) for i in range(30000)]


def text():
    return " ".join(rng.choices(vocab, weights, k=WORDS))


def append(srv, rows):
    for i in range(0, len(rows), BATCH):
        ev = [upsert(k, v.encode(), f"{k}:{n}") for n, (k, v) in enumerate(rows[i:i + BATCH], i)]
        srv.req("POST", "/db/r/append", b"".join(ev), "application/x-ikam-events")


def timed(srv):
    t = time.perf_counter()
    hits = json.loads(srv.req("GET", Q))
    return round(time.perf_counter() - t, 3), [h["key"] for h in hits]


store = os.path.join(WORK, "store")
shutil.rmtree(store, ignore_errors=True)
rows = [(f"t/{i:05}", text()) for i in range(ROWS)]
srv = Server(store)
t = time.perf_counter()
append(srv, rows)
out(step="load", rows=ROWS, text_mb=round(sum(len(v) for _, v in rows) / 1e6, 1),
    s=round(time.perf_counter() - t, 1), store_mb=round(du(store) / 1e6, 1), flags=FLAGS)
s, top = timed(srv)
out(step="first query", s=s, store_mb=round(du(store) / 1e6, 1), rss_mb=srv.rss_mb())
first = top
srv.stop()

srv = Server(store)
s, top = timed(srv)
out(step="restart, no change", s=s, store_mb=round(du(store) / 1e6, 1), rss_mb=srv.rss_mb(),
    same=top == first)
again, _ = timed(srv)
out(step="next query", s=again)
srv.stop()

# 1% of the rows rewritten by a server that searches nothing.
srv = Server(store)
changed = [(f"t/{i:05}", text()) for i in rng.sample(range(ROWS), ROWS // 100)]
append(srv, changed)
srv.stop()
srv = Server(store)
s, top = timed(srv)
out(step="restart after 1% rewritten", s=s, store_mb=round(du(store) / 1e6, 1),
    rss_mb=srv.rss_mb())
after = top
srv.stop()

srv = Server(store)
s, top = timed(srv)
out(step="restart, no change again", s=s, store_mb=round(du(store) / 1e6, 1), same=top == after)
srv.stop()
