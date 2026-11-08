#!/usr/bin/env python3
"""E110 live bench: drive `ikam live` the way a person would and check it.

    python3 -I realworld/live_bench.py --ikam target/release/ikam \
        --corpora ~/ikam-corpora --work /tmp/live-bench [--steps 400] [--seed 1]

Phases, each against a fresh `ikam live` on a fresh store:

1. upload: every corpus file PUT over WebDAV, one at a time. Per file: the
   request's latency and the bytes it added (the server's X-Ikam-Cost
   header), by kind. Then commit, and compare the store with what a repack
   makes of the same files (how far edits drift from the repacked size).
2. edits: a random sequence of edits (change, append, new file, copy,
   rename, delete, new folder, folder rename) checked against a copy of the
   tree kept here. After every edit the touched paths are read back; every
   50 edits the whole tree is listed and every file read back.
3. queries: search latency, and for plain text files whether search finds
   exactly the files that contain the term (the index follows edits).
4. watch: a folder mirrored into the tree; time from a change on disk to
   the change being readable over WebDAV.
5. restart: commit, stop, start again; startup time, then every file read
   back, and fsck.

Prints one JSON object. Nothing here needs a network beyond 127.0.0.1.
"""

import argparse
import hashlib
import http.client
import json
import os
import random
import shutil
import statistics
import subprocess
import sys
import time
import urllib.parse

TEXT = (".py", ".txt", ".md", ".csv", ".json", ".tsv", ".rst", ".toml", ".cfg", ".ini")


def kind_of(path):
    p = path.lower()
    for k in (".tar.gz", ".whl", ".docx", ".pptx", ".xlsx", ".pdf", ".odt", ".ods"):
        if p.endswith(k):
            return k[1:]
    return "text" if p.endswith(TEXT) else "other"


class Server:
    def __init__(self, ikam, store, port, extra=()):
        self.args = [ikam, "--store", store, "live", str(port), "--commit-after", "300", *extra]
        self.port = port
        self.log = open(store + ".log", "ab")
        try:
            self.conn()
            sys.exit(f"port {port} is already in use")
        except OSError:
            pass
        t = time.time()
        self.proc = subprocess.Popen(self.args, stderr=self.log, stdout=self.log)
        SERVERS.append(self)
        while True:
            try:
                self.conn().request("GET", "/api/status")
                break
            except OSError:
                if self.proc.poll() is not None:
                    sys.exit("ikam live exited; see " + store + ".log")
                time.sleep(0.05)
        self.startup_s = time.time() - t
        self.c = self.conn()

    def conn(self):
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=600)
        c.connect()
        return c

    def req(self, method, path, body=None, headers=None):
        h = {"Host": f"127.0.0.1:{self.port}", "X-Ikam": "1"}
        h.update(headers or {})
        for attempt in range(2):
            try:
                self.c.request(method, path, body=body, headers=h)
                r = self.c.getresponse()
                return r.status, dict((k.lower(), v) for k, v in r.getheaders()), r.read()
            except (http.client.HTTPException, OSError):
                if attempt:
                    raise
                self.c = self.conn()

    def dav(self, method, path, body=None, headers=None):
        return self.req(method, "/dav/" + urllib.parse.quote(path), body, headers)

    def api(self, name, method="GET"):
        s, _, b = self.req(method, "/api/" + name)
        if s != 200:
            raise RuntimeError(f"/api/{name}: {s} {b[:200]!r}")
        return json.loads(b) if b[:1] in (b"{", b"[") else b.decode()

    def stop(self):
        self.proc.terminate()
        self.proc.wait()


def cost(headers):
    return dict(kv.split("=") for kv in headers.get("x-ikam-cost", "").split())


def pct(xs, q):
    if not xs:
        return None
    xs = sorted(xs)
    return round(xs[min(len(xs) - 1, int(q * len(xs)))], 2)


def summary(xs):
    return {"n": len(xs), "p50": pct(xs, 0.5), "p90": pct(xs, 0.9), "max": pct(xs, 1.0)}


def corpus_files(root, skip):
    out = []
    for d, _, fs in os.walk(root):
        for f in fs:
            full = os.path.join(d, f)
            rel = os.path.relpath(full, root).replace(os.sep, "/")
            if rel.split("/")[0] in skip or rel == "MANIFEST.tsv":
                continue
            out.append((rel, full))
    return sorted(out)


def mkcols(srv, path, made):
    parts = path.split("/")[:-1]
    for i in range(1, len(parts) + 1):
        d = "/".join(parts[:i])
        if d not in made:
            srv.dav("MKCOL", d)
            made.add(d)


def listing(srv):
    """Every file under /dav/ with its size, by PROPFIND."""
    files, todo = {}, [""]
    while todo:
        d = todo.pop()
        s, _, b = srv.dav("PROPFIND", d, headers={"Depth": "1"})
        assert s == 207, (d, s)
        text = b.decode()
        for resp in text.split("<D:response>")[1:]:
            href = resp.split("<D:href>")[1].split("</D:href>")[0]
            p = urllib.parse.unquote(href)[len("/dav/"):].rstrip("/")
            if p == d:
                continue
            if "<D:collection/>" in resp:
                todo.append(p)
            else:
                files[p] = int(resp.split("<D:getcontentlength>")[1].split("<")[0])
    return files


def check_all(srv, shadow):
    files = listing(srv)
    bad = sorted(set(files) ^ set(shadow))
    for p, data in shadow.items():
        if p in files:
            s, _, b = srv.dav("GET", p)
            if s != 200 or b != data:
                bad.append(p)
    return bad


def upload_phase(srv, files):
    by_kind, lat, made = {}, [], set()
    for rel, full in files:
        data = open(full, "rb").read()
        mkcols(srv, rel, made)
        t = time.perf_counter()
        s, h, _ = srv.dav("PUT", rel, data)
        ms = (time.perf_counter() - t) * 1e3
        assert s in (201, 204), (rel, s)
        c = cost(h)
        k = by_kind.setdefault(kind_of(rel), {"files": 0, "bytes": 0, "new_bytes": 0, "ms": []})
        k["files"] += 1
        k["bytes"] += len(data)
        k["new_bytes"] += int(c["new_bytes"])
        k["ms"].append(ms)
        lat.append(ms)
    for k in by_kind.values():
        k["ms"] = summary(k["ms"])
        k["ratio"] = round(k["new_bytes"] / max(1, k["bytes"]), 4)
    return by_kind, lat


def edit_phase(srv, shadow, rng, steps):
    lat = {}
    folders = {p.rsplit("/", 1)[0] for p in shadow if "/" in p}
    bad, checks = [], 0

    def do(kind, method, path, body=None, headers=None, ok=(200, 201, 204)):
        t = time.perf_counter()
        s, h, _ = srv.dav(method, path, body, headers)
        ms = (time.perf_counter() - t) * 1e3
        assert s in ok, (kind, method, path, s)
        lat.setdefault(kind, {"ms": [], "new_bytes": []})
        lat[kind]["ms"].append(ms)
        lat[kind]["new_bytes"].append(int(cost(h).get("new_bytes", 0)))

    def dest(p):
        return {"Destination": "/dav/" + urllib.parse.quote(p), "Overwrite": "T"}

    for step in range(steps):
        texts = [p for p in shadow if p.endswith(TEXT)]
        op = rng.choice(["change", "change", "append", "new", "copy", "rename", "delete", "mkdir", "dirmove"])
        touched = []
        if op in ("change", "append") and texts:
            p = rng.choice(texts)
            lines = shadow[p].split(b"\n")
            if op == "change":
                i = rng.randrange(len(lines))
                lines[i] = b"edited %d: %s" % (step, lines[i][:40])
            else:
                lines.append(b"appended line %d" % step)
            data = b"\n".join(lines)
            kind = op + ("-large" if len(data) > 100_000 else "")
            do(kind, "PUT", p, data)
            shadow[p] = data
            touched = [p]
        elif op == "new":
            d = rng.choice(sorted(folders) or [""])
            p = (d + "/" if d else "") + f"note-{step}.md"
            data = (f"# Note {step}\n\nwritten by the live bench, term zebra{step % 7}\n").encode()
            do("new", "PUT", p, data)
            shadow[p] = data
            touched = [p]
        elif op in ("copy", "rename", "delete") and shadow:
            p = rng.choice(sorted(shadow))
            if op == "delete":
                do("delete", "DELETE", p)
                del shadow[p]
            else:
                q = p.rsplit(".", 1)[0] + f"-{op}{step}." + p.rsplit(".", 1)[-1]
                do(op, "COPY" if op == "copy" else "MOVE", p, headers=dest(q))
                shadow[q] = shadow[p]
                if op == "rename":
                    del shadow[p]
                touched = [q]
        elif op == "mkdir":
            d = f"made-{step}"
            do("mkdir", "MKCOL", d, ok=(201,))
            folders.add(d)
        elif op == "dirmove" and folders:
            d = rng.choice(sorted(folders))
            n = d + f"-moved{step}"
            inside = [p for p in shadow if p.startswith(d + "/")]
            if not inside or len(inside) > 40:
                continue
            do("dirmove", "MOVE", d, headers=dest(n))
            for p in inside:
                shadow[n + p[len(d):]] = shadow.pop(p)
            folders = {f for f in folders if f != d and not f.startswith(d + "/")}
            folders.add(n)
            touched = [n + p[len(d):] for p in inside]
        for p in touched:
            s, _, b = srv.dav("GET", p)
            if s != 200 or b != shadow[p]:
                bad.append((step, op, p))
        if (step + 1) % 50 == 0:
            checks += 1
            bad += [(step, "full", p) for p in check_all(srv, shadow)]
    out = {k: {"ms": summary(v["ms"]), "new_bytes": summary(v["new_bytes"])} for k, v in sorted(lat.items())}
    return out, bad, checks


def query_phase(srv, shadow, rng):
    words = set()
    for p in sorted(shadow):
        if p.endswith(TEXT):
            for w in shadow[p][:4000].split():
                if 5 <= len(w) <= 12 and w.isalpha():
                    words.add(w.decode())
    terms = rng.sample(sorted(words), min(40, len(words))) + [f"zebra{i}" for i in range(7)]
    ms, wrong = [], []
    for t in terms:
        r = srv.api("search?q=" + urllib.parse.quote(t))
        ms.append(r["ms"])
        got = {h["path"] for h in r["hits"] if not h["part"] and h["path"].endswith(TEXT)}
        want = {p for p, d in shadow.items() if p.endswith(TEXT) and t.lower().encode() in d.lower()}
        if got != want:
            wrong.append({"term": t, "missing": sorted(want - got)[:3], "extra": sorted(got - want)[:3]})
    questions = [
        "how do I add a paragraph to a document",
        "what does the command line option parser do",
        "which data set lists cars and their horsepower",
        "how are tables merged in a spreadsheet",
    ]
    ask = [srv.api("ask?q=" + urllib.parse.quote(q))["ms"] for q in questions]
    return {"search_ms": summary(ms), "terms": len(terms), "wrong": wrong, "ask_retrieval_ms": summary(ask)}


def watch_phase(ikam, work, port, rng):
    store, wdir = os.path.join(work, "watch-store"), os.path.join(work, "watched")
    os.makedirs(wdir)
    for i in range(20):
        open(os.path.join(wdir, f"f{i}.txt"), "w").write(f"file {i}\n" * 50)
    srv = Server(ikam, store, port, ["--watch", wdir + ":inbox"])
    lag, bad = [], []

    def wait(path, want):
        t = time.perf_counter()
        while time.perf_counter() - t < 15:
            s, _, b = srv.dav("GET", "inbox/" + path)
            if (want is None and s == 404) or (want is not None and s == 200 and b == want):
                return time.perf_counter() - t
            time.sleep(0.02)
        bad.append(path)
        return None

    wait("f19.txt", b"file 19\n" * 50)
    for step in range(30):
        i = rng.randrange(25)
        p = os.path.join(wdir, f"f{i}.txt")
        if os.path.exists(p) and rng.random() < 0.3:
            t0 = time.perf_counter()
            os.remove(p)
            w = wait(f"f{i}.txt", None)
        else:
            data = (f"file {i} step {step}\n" * rng.randrange(1, 200)).encode()
            t0 = time.perf_counter()
            open(p, "wb").write(data)
            w = wait(f"f{i}.txt", data)
        if w is not None:
            lag.append((time.perf_counter() - t0) * 1e3)
    srv.stop()
    return {"lag_ms": summary(lag), "missed": bad}


SERVERS = []


def main():
    try:
        run()
    finally:
        for s in SERVERS:
            if s.proc.poll() is None:
                s.proc.terminate()


def run():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ikam", required=True)
    ap.add_argument("--corpora", required=True)
    ap.add_argument("--work", required=True)
    ap.add_argument("--port", type=int, default=8790)
    ap.add_argument("--steps", type=int, default=400)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--skip", default="pandas-sdists", help="corpus folders to leave out, comma separated")
    a = ap.parse_args()
    if os.path.exists(a.work):
        shutil.rmtree(a.work)
    os.makedirs(a.work)
    rng = random.Random(a.seed)
    ikam = os.path.abspath(a.ikam)
    files = corpus_files(a.corpora, set(a.skip.split(",")))
    out = {"files": len(files), "bytes": sum(os.path.getsize(f) for _, f in files)}

    store = os.path.join(a.work, "store")
    srv = Server(ikam, store, a.port)
    t = time.time()
    out["upload"], lat = upload_phase(srv, files)
    out["upload_total_s"] = round(time.time() - t, 1)
    out["upload_ms"] = summary(lat)
    srv.api("commit", "POST")
    out["store_bytes_after_upload"] = srv.api("status")["store_bytes"]
    t = time.time()
    out["repack_after_upload"] = srv.api("repack", "POST")
    out["repack_after_upload_s"] = round(time.time() - t, 1)

    shadow = {rel: open(full, "rb").read() for rel, full in files}
    t = time.time()
    out["edits"], bad, checks = edit_phase(srv, shadow, rng, a.steps)
    out["edit_total_s"] = round(time.time() - t, 1)
    out["edit_mismatches"] = bad[:20]
    out["edit_mismatch_count"] = len(bad)
    out["full_checks"] = checks
    out["queries"] = query_phase(srv, shadow, rng)

    srv.api("commit", "POST")
    st = srv.api("status")
    out["store_bytes_after_edits"] = st["store_bytes"]
    t = time.time()
    out["repack"] = srv.api("repack", "POST")
    out["repack_s"] = round(time.time() - t, 1)
    out["store_bytes_after_repack_on_disk"] = srv.api("status")["store_bytes"]
    srv.stop()

    srv = Server(ikam, store, a.port)
    out["restart_s"] = round(srv.startup_s, 2)
    out["after_restart_mismatches"] = check_all(srv, shadow)[:20]
    out["fsck"] = srv.api("fsck", "POST")
    srv.stop()

    out["watch"] = watch_phase(ikam, a.work, a.port + 1, rng)
    print(json.dumps(out, indent=1))
    ok = not bad and not out["after_restart_mismatches"] and out["fsck"].endswith("=0") and not out["queries"]["wrong"]
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
