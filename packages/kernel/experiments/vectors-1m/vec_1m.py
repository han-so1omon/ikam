"""E389: vector search through `ikam db serve` as a table grows to 1M rows.

    python3 -I vec_1m.py WORK OUT_JSONL CHECKS "MODE;MODE;..." [QUERIES]

CHECKS is a comma list of row counts (100000,250000,500000,1000000). The
rows are `v/0000000`... holding 384 little-endian f32s each, unit vectors
in 1,000 clusters (a centroid plus noise, cosine about 0.7 to it), the
shape of sentence embeddings of a mixed corpus. The table is loaded with
one server in 2,000-row appends up to each CHECK, then each MODE gets a
fresh server on the same store and is asked QUERIES (default 200) queries
on `POST /db/v/vsearch?prefix=v/&k=10`: the first one builds the server's
index (vector indexes are not kept, so a restarted server builds again),
the rest are timed one by one. A MODE is `LABEL[@N,N]=BIN:FLAGS`; with
`@N,N` it runs only at those CHECKs. Recall@10 is against the exact top
10 computed here with numpy, independently of the server.

Prints and appends one JSON line per CHECK and MODE: server start, first
query (the build), memory after it (VmRSS) and at its peak (VmHWM), query
p50/p99 and recall@10. Synthetic rows only; WORK is scratch; 127.0.0.1.
Linux (/proc).
"""
import base64, http.client, json, os, shutil, socket, struct, subprocess, sys, time

import numpy as np

DIM, CLUSTERS, NOISE, BATCH, PW = 384, 1000, 0.05, 2000, "e389-test"


def centroids():
    c = np.random.default_rng(3890).standard_normal((CLUSTERS, DIM))
    return (c / np.linalg.norm(c, axis=1, keepdims=True)).astype(np.float32)


def rows(cents, start, n, seed):
    rng = np.random.default_rng([seed, start])
    v = cents[rng.integers(0, CLUSTERS, n)] + NOISE * rng.standard_normal((n, DIM)).astype(np.float32)
    return (v / np.linalg.norm(v, axis=1, keepdims=True)).astype(np.float32)


def pack(*parts):
    return b"".join(struct.pack(">I", len(p)) + p for p in parts)


def mem(pid):
    out = {}
    for line in open("/proc/%d/status" % pid):
        k = line.split(":")[0]
        if k in ("VmRSS", "VmHWM"):
            out[k.lower() + "_mb"] = round(int(line.split()[1]) / 1024)
    return out


def du(path):
    return sum(os.path.getsize(os.path.join(r, f)) for r, _, fs in os.walk(path) for f in fs)


class Server:
    def __init__(self, binary, store, flags, log):
        s = socket.socket(); s.bind(("127.0.0.1", 0)); self.port = s.getsockname()[1]; s.close()
        line = subprocess.run([binary, "passwd", "u"], input=PW + "\n", capture_output=True,
                              text=True, check=True).stdout.strip()
        self.hdr = {"Authorization": "Basic " + base64.b64encode(("u:" + PW).encode()).decode()}
        self.log = open(log, "ab")
        t = time.time()
        self.p = subprocess.Popen([binary, "--store", store, "db", "serve", *flags, str(self.port)],
                                  env=dict(os.environ, IKAM_AUTH=line.split("=", 1)[1]),
                                  stdout=self.log, stderr=self.log)
        while True:
            try:
                self.c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=7200)
                self.call("GET", "/db/v/head")
                break
            except OSError:
                if time.time() - t > 600 or self.p.poll() is not None:
                    raise RuntimeError("db serve did not start")
                time.sleep(0.1)
        self.ready = time.time() - t

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

    def append(self, start, vecs):
        out = [b"\x00" + pack(b"v/%07d" % (start + i), b"e%d" % (start + i), v.tobytes())
               for i, v in enumerate(vecs)]
        self.call("POST", "/db/v/append", b"".join(out), "application/x-ikam-events")

    def vsearch(self, q):
        b = self.call("POST", "/db/v/vsearch?prefix=v/&k=10", q.tobytes())
        return [int(h["key"][2:]) for h in json.loads(b)]

    def stop(self):
        self.c.close(); self.p.terminate()
        try:
            self.p.wait(120)
        except subprocess.TimeoutExpired:
            self.p.kill(); self.p.wait()
        self.log.close()


def truth(mm, n, qs):
    best = np.full((len(qs), 10), -np.inf, dtype=np.float32)
    ids = np.zeros((len(qs), 10), dtype=np.int64)
    for s in range(0, n, 100000):
        block = np.asarray(mm[s:min(n, s + 100000)])
        sc = qs @ block.T
        top = np.argpartition(-sc, 10, axis=1)[:, :10]
        cand = np.concatenate([best, np.take_along_axis(sc, top, 1)], 1)
        cid = np.concatenate([ids, top + s], 1)
        keep = np.argsort(-cand, axis=1)[:, :10]
        best, ids = np.take_along_axis(cand, keep, 1), np.take_along_axis(cid, keep, 1)
    return [set(r) for r in ids.tolist()]


def pct(xs, p):
    xs = sorted(xs)
    return round(xs[min(len(xs) - 1, int(len(xs) * p))] * 1e3, 2)


def main():
    work, out_path = sys.argv[1], sys.argv[2]
    checks = [int(x) for x in sys.argv[3].split(",")]
    modes = []
    for m in sys.argv[4].split(";"):
        label, rest = m.split("=", 1)
        only = None
        if "@" in label:
            label, at = label.split("@")
            only = {int(x) for x in at.split(",")}
        binary, flags = rest.split(":", 1)
        modes.append((label, only, binary, flags.split()))
    nq = int(sys.argv[5]) if len(sys.argv) > 5 else 200
    shutil.rmtree(work, ignore_errors=True); os.makedirs(work)
    store, log = os.path.join(work, "store"), os.path.join(work, "db.log")
    cents = centroids()
    qs = rows(cents, 0, nq, 38900)
    mm = np.lib.format.open_memmap(os.path.join(work, "rows.npy"), "w+", np.float32,
                                   (checks[-1], DIM))
    loaded, out = 0, open(out_path, "a")
    for n in checks:
        loader = Server(modes[0][2], store, [], log)
        t = time.time()
        for s in range(loaded, n, BATCH):
            v = rows(cents, s, min(BATCH, n - s), 389)
            mm[s:s + len(v)] = v
            loader.append(s, v)
        load_s, loaded = time.time() - t, n
        loader.stop(); mm.flush()
        want = truth(mm, n, qs)
        for label, only, binary, flags in modes:
            if only is not None and n not in only:
                continue
            s = Server(binary, store, flags, log)
            t = time.time()
            first = s.vsearch(qs[0])
            build = time.time() - t
            after = mem(s.p.pid)
            times, hit = [], len(set(first) & want[0])
            for i in range(1, nq):
                t = time.perf_counter()
                got = s.vsearch(qs[i])
                times.append(time.perf_counter() - t)
                hit += len(set(got) & want[i])
            res = dict(rows=n, mode=label, flags=" ".join(flags), load_s=round(load_s),
                       store_mb=round(du(store) / 1e6), ready_s=round(s.ready, 1),
                       first_query_s=round(build, 1), **after, end_rss_mb=mem(s.p.pid)["vmrss_mb"],
                       p50_ms=pct(times, 0.5), p99_ms=pct(times, 0.99),
                       recall_at_10=round(hit / (10 * nq), 4))
            s.stop()
            line = json.dumps(res)
            print(line, flush=True)
            out.write(line + "\n"); out.flush()


main()
