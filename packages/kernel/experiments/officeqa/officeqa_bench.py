#!/usr/bin/env python3
"""E500: does ikam's served search find the pages OfficeQA Pro V2's questions
are answered from, against a plain retriever on the same questions?

    python3 -I officeqa_bench.py IKAM_BIN WORK_DIR DATA_DIR [FLAGS...]

DATA_DIR comes from prepare.py. Every page is stored as a row `p/STEM/PAGE`
in `db serve` (FLAGS, default `--live`), in 2,000-row batches; with
pages.f32 present, its vector as `v/STEM/PAGE`. Each question's text is the
query, as an agent would first send it. Rankings of the top 100 pages:
- grep-words: pages by how many distinct query terms they hold, ties in key
  order (what a file-search agent's grep per word gives);
- plain-bm25: BM25 in this script, Lucene's defaults (k1 1.2, b 0.75), the
  same idf, terms as `\\w+` lowercased: a baseline outside ikam;
- ikam-bm25: `GET search` (the kernel's BM25: K1 0.9, B 0.4, alphanumeric
  terms);
- with vectors: ikam-vectors (`vsearch`) and ikam-fused (`search&vectors=`,
  convex at 0.5 and 0.7, and RRF).
Scores per question: page recall@k (share of its gold pages in the top k),
page hit@k (any gold page), doc recall@k and all-docs@k over the documents
in the order their first page ranks, MRR of the first gold page, and page
recall@10 allowing one page either side (tables run across pages). Means
over the questions, and paired differences against plain-bm25 with their
standard error. Prints one JSON line a measurement and writes each
question's scores to WORK_DIR/perq.jsonl. Nothing from the data is
committed.
"""
import array
import collections
import http.client
import json
import math
import os
import re
import shutil
import socket
import struct
import subprocess
import sys
import time
import urllib.parse

BIN, WORK, DATA = sys.argv[1], sys.argv[2], sys.argv[3]
FLAGS = sys.argv[4:] or ["--live"]
BATCH, DEPTH, KS, RS = 2000, 100, (1, 5, 10, 20, 50, 100), "\x1e"


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
        self.store, self.flags, self.port = store, flags, port_free()

    def start(self):
        log = open(os.path.join(WORK, "db.log"), "ab")
        self.proc = subprocess.Popen([BIN, "--store", self.store, "db", "serve", *self.flags,
                                      str(self.port)], stdout=log, stderr=log)
        for _ in range(200):
            try:
                self.conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=1800)
                self.req("GET", "/db/")
                return
            except OSError:
                time.sleep(0.05)
        raise SystemExit("server did not start")

    def stop(self):
        self.proc.terminate()
        self.proc.wait()

    def rss_mb(self):
        try:
            for line in open(f"/proc/{self.proc.pid}/status"):
                if line.startswith("VmRSS:"):
                    return round(int(line.split()[1]) / 1024)
        except OSError:  # no /proc (macOS)
            r = subprocess.run(["ps", "-o", "rss=", "-p", str(self.proc.pid)],
                               capture_output=True, text=True)
            return round(int(r.stdout.strip() or 0) / 1024)

    def req(self, method, path, body=None, ctype=None):
        h = {"Host": f"127.0.0.1:{self.port}"}
        if ctype:
            h["Content-Type"] = ctype
        try:
            self.conn.request(method, path, body=body, headers=h)
            r = self.conn.getresponse()
        except (http.client.RemoteDisconnected, BrokenPipeError, ConnectionResetError):
            # The server closes a connection idle for 2 minutes.
            self.conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=1800)
            self.conn.request(method, path, body=body, headers=h)
            r = self.conn.getresponse()
        data = r.read()
        if r.status != 200:
            raise RuntimeError(f"{method} {path[:80]}: {r.status} {data[:200]!r}")
        return data


class PlainBm25:
    """Okapi BM25 with Lucene's defaults, postings in flat arrays."""

    def __init__(self, texts, k1=1.2, b=0.75):
        self.k1, self.b, self.n = k1, b, len(texts)
        post = collections.defaultdict(lambda: (array.array("I"), array.array("H")))
        self.lens = array.array("I")
        for i, t in enumerate(texts):
            tf = collections.Counter(re.findall(r"\w+", t.lower()))
            self.lens.append(sum(tf.values()))
            for term, c in tf.items():
                docs, tfs = post[term]
                docs.append(i)
                tfs.append(min(c, 65535))
        self.post, self.avg = dict(post), sum(self.lens) / max(1, self.n)

    def terms(self, q):
        return list(dict.fromkeys(re.findall(r"\w+", q.lower())))

    def top(self, q, k):
        score = collections.defaultdict(float)
        for term in self.terms(q):
            if term not in self.post:
                continue
            docs, tfs = self.post[term]
            idf = math.log(1 + (self.n - len(docs) + 0.5) / (len(docs) + 0.5))
            for d, tf in zip(docs, tfs):
                norm = self.k1 * (1 - self.b + self.b * self.lens[d] / self.avg)
                score[d] += idf * tf * (self.k1 + 1) / (tf + norm)
        return sorted(score, key=lambda d: (-score[d], d))[:k]

    def coordination(self, q, k):
        """Pages by distinct query terms held, ties in key order."""
        held = collections.Counter()
        for term in self.terms(q):
            held.update(self.post.get(term, ((), ()))[0])
        return sorted(held, key=lambda d: (-held[d], d))[:k]


def scores(ranked, q):
    """One question's scores for a ranking of page keys (no `p/`)."""
    pages, docs = q["pages"], q["docs"]
    order = list(dict.fromkeys(k.rsplit("/", 1)[0] for k in ranked))
    near = {f"{k.rsplit('/', 1)[0]}/{int(k.rsplit('/', 1)[1]) + d}"
            for k in ranked[:10] for d in (-1, 0, 1)}
    s = {"mrr": next((1 / (i + 1) for i, k in enumerate(ranked) if k in pages), 0.0),
         "page_recall@10±1": sum(p in near for p in pages) / len(pages) if pages else None}
    for k in KS:
        top = set(ranked[:k])
        s[f"page_recall@{k}"] = len(top & pages) / len(pages) if pages else None
        s[f"page_hit@{k}"] = float(bool(top & pages)) if pages else None
        if k <= 20:
            got = set(order[:k]) & docs
            s[f"doc_recall@{k}"] = len(got) / len(docs) if docs else None
            s[f"all_docs@{k}"] = float(got == docs) if docs else None
    return s


def mean(xs):
    xs = [x for x in xs if x is not None]
    return round(sum(xs) / len(xs), 4) if xs else None


def summary(runs, base=None):
    names = runs[0].keys()
    res = {n: mean([r[n] for r in runs]) for n in names}
    if base is not None:
        for n in ("page_recall@10", "page_recall@100", "doc_recall@10", "mrr"):
            d = [a[n] - b[n] for a, b in zip(runs, base) if a[n] is not None]
            m = sum(d) / len(d)
            se = math.sqrt(sum((x - m) ** 2 for x in d) / (len(d) - 1) / len(d))
            res[f"diff {n}"] = f"{m:+.4f} ({se:.4f})"
    return res


def pct(xs, p):
    xs = sorted(xs)
    return round(xs[min(len(xs) - 1, int(p * len(xs)))] * 1000, 2)


def du(p):
    return sum(os.path.getsize(os.path.join(d, f)) for d, _, fs in os.walk(p) for f in fs)


def main():
    keys, texts = [], []
    for line in open(os.path.join(DATA, "pages.tsv"), encoding="utf-8"):
        key, text = line.rstrip("\n").split("\t", 1)
        keys.append(key)
        texts.append(text.replace(RS, "\n"))
    queries = []
    for line in open(os.path.join(DATA, "queries.tsv"), encoding="utf-8"):
        uid, pages, docs, text = line.rstrip("\n").split("\t", 3)
        queries.append({"uid": uid, "pages": set(filter(None, pages.split(","))),
                        "docs": set(filter(None, docs.split(","))), "text": text})
    vecs = None
    if os.path.exists(os.path.join(DATA, "pages.f32")):
        dim = json.load(open(os.path.join(DATA, "vectors.json")))["dim"]
        raw = open(os.path.join(DATA, "pages.f32"), "rb").read()
        vecs = [raw[i * dim * 4:(i + 1) * dim * 4] for i in range(len(keys))]
        qraw = open(os.path.join(DATA, "queries.f32"), "rb").read()
        qvecs = [qraw[i * dim * 4:(i + 1) * dim * 4] for i in range(len(queries))]
    out(pages=len(keys), docs=len({k.rsplit("/", 1)[0] for k in keys}), queries=len(queries),
        text_mb=round(sum(len(t.encode()) for t in texts) / 1e6, 1), flags=FLAGS,
        vectors=vecs is not None)

    shutil.rmtree(WORK, ignore_errors=True)
    os.makedirs(WORK)
    store = os.path.join(WORK, "store")
    srv = Server(store, FLAGS)
    srv.start()
    t = time.perf_counter()
    for s in range(0, len(keys), BATCH):
        ev = []
        for i in range(s, min(s + BATCH, len(keys))):
            ev.append(upsert("p/" + keys[i], texts[i].encode(), f"p{i}"))
            if vecs:
                ev.append(upsert("v/" + keys[i], vecs[i], f"v{i}"))
        srv.req("POST", "/db/q/append", b"".join(ev), "application/x-ikam-events")
    out(step="load", s=round(time.perf_counter() - t, 1), store_mb=round(du(store) / 1e6, 1),
        rss_mb=srv.rss_mb())

    def search(q):
        return "/db/q/search?" + urllib.parse.urlencode({"q": q, "prefix": "p/", "k": DEPTH})

    def hits(body, pre):
        return [h["key"][len(pre):] for h in json.loads(body)]

    before = du(store)
    t = time.perf_counter()
    srv.req("GET", search(queries[0]["text"]))
    out(step="first-query", bm25_build_s=round(time.perf_counter() - t, 2),
        view_written_mb=round((du(store) - before) / 1e6, 1), rss_mb=srv.rss_mb())

    t = time.perf_counter()
    plain = PlainBm25(texts)
    out(step="plain-bm25-build", s=round(time.perf_counter() - t, 1), terms=len(plain.post))

    rankings = {
        "plain-bm25": lambda i, q: [keys[d] for d in plain.top(q["text"], DEPTH)],
        "grep-words": lambda i, q: [keys[d] for d in plain.coordination(q["text"], DEPTH)],
        "ikam-bm25": lambda i, q: hits(srv.req("GET", search(q["text"])), "p/"),
    }
    if vecs:
        rankings["ikam-vectors"] = lambda i, q: hits(srv.req(
            "POST", f"/db/q/vsearch?prefix=v/&k={DEPTH}", qvecs[i]), "v/")
        for fuse in ("alpha=0.5", "alpha=0.7", "fuse=rrf"):
            rankings["ikam-fused " + fuse] = (lambda f: lambda i, q: hits(srv.req(
                "POST", search(q["text"]) + "&vectors=v/&" + f, qvecs[i]), "p/"))(fuse)

    runs = {}
    with open(os.path.join(WORK, "perq.jsonl"), "w") as perq:
        for name, fn in rankings.items():
            runs[name], secs = [], []
            for i, q in enumerate(queries):
                t = time.perf_counter()
                ranked = fn(i, q)
                secs.append(time.perf_counter() - t)
                runs[name].append(scores(ranked, q))
                perq.write(json.dumps({"ranking": name, "uid": q["uid"], **runs[name][-1]}) + "\n")
            base = runs["plain-bm25"] if name != "plain-bm25" else None
            out(ranking=name, latency_ms={"p50": pct(secs, 0.5), "p95": pct(secs, 0.95)},
                **summary(runs[name], base))
    out(step="end", store_mb=round(du(store) / 1e6, 1), rss_mb=srv.rss_mb())
    srv.stop()


if __name__ == "__main__":
    main()
