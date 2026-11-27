#!/usr/bin/env python3
"""E393: `ikam db serve` under mixed traffic at 1M rows.

Loads ROWS indexed rows (E354's rows, 2,000-row batches), warms the BM25
view with one search, then runs MIX seconds of four clients at once:
an appender (2,000 new rows a batch), an editor (500 upserts of old rows a
batch, each a near-copy of the row it replaces), a reader (point gets of random old rows) and a searcher (one ranked
search a second). Records each op's latency, resident memory every 10 s, and
after a restart a full count, then runs `ikam fsck` on the store.

    python3 -I mixed_load.py IKAM_BIN WORK ROWS MIX "FLAGS;FLAGS"

Prints one JSON line per flag set. Synthetic rows; 127.0.0.1 only.
"""
import json, os, random, shutil, subprocess, sys, threading, time
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "goal-scorecard"))
import db_first_write as d

BATCH, EDITS = 2000, 500


def rss(pid):
    for line in open("/proc/%d/status" % pid):
        if line.startswith("VmRSS"):
            return round(int(line.split()[1]) / 1024)


def pct(xs, p):
    if not xs:
        return None
    xs = sorted(xs)
    return round(xs[min(len(xs) - 1, int(p * len(xs)))] * 1000, 1)


def summary(lat):
    return {"n": len(lat), "p50_ms": pct(lat, 0.5), "p99_ms": pct(lat, 0.99),
            "max_ms": pct(lat, 1.0)}


def row_values(start, n):
    """The values `d.events` gave rows start..start+n (one load batch)."""
    rnd = random.Random(start)
    return [[i, "2026-01-%02d" % (1 + i % 28), "word%d" % rnd.randrange(5000),
             rnd.random() * 1000, "x" * rnd.randrange(40, 160)] for i in range(start, start + n)]


def edits(rows, n, run, rnd):
    """`n` edits of old rows of one load batch: each its row with a new
    amount and a note, so a near-copy of what it replaces."""
    start = rnd.randrange(rows // BATCH) * BATCH
    vals = row_values(start, BATCH)
    out = []
    for j in range(n):
        k = rnd.randrange(BATCH)
        v = list(vals[k])
        v[3] = rnd.random() * 1000
        v.append("edited %s %d" % (run.decode(), j))
        pk = b"p%019d" % (start + k)
        out.append(b"\x00" + d.pack(b"t/big/" + pk, b"%s-%d" % (run, j), json.dumps(v).encode()))
    return b"".join(out)


def one(binary, work, rows, mix, flags):
    w = os.path.join(work, "m")
    shutil.rmtree(w, ignore_errors=True)
    os.makedirs(w)
    store = os.path.join(w, "store")
    s = d.Server(binary, store, flags.split())
    s.start()
    t0 = time.time()
    for start in range(0, rows, BATCH):
        n = min(BATCH, rows - start)
        assert s.append(d.events(b"big", start, n, b"load"), 1800)[1]
    load_s = time.time() - t0
    t1 = time.time()
    s.req("GET", "/db/t/search?q=word7&prefix=t/big/&k=10", None, 1800)
    warm_s = time.time() - t1
    rss0 = rss(s.proc.pid)
    lat = {"append": [], "edit": [], "get": [], "search": []}
    fails = {k: 0 for k in lat}
    stop = time.time() + mix
    nxt = [rows]

    def loop(kind, body, pause=0.0):
        rnd = random.Random(kind)
        k = 0
        while time.time() < stop:
            t = time.time()
            try:
                body(rnd, k)
                lat[kind].append(time.time() - t)
            except Exception:
                fails[kind] += 1
            k += 1
            if pause:
                time.sleep(pause)

    def app(rnd, k):
        start = nxt[0]
        nxt[0] += BATCH
        s.req("POST", "/db/t/append", d.events(b"big", start, BATCH, b"mix"), 600)

    def edit(rnd, k):
        s.req("POST", "/db/t/append", edits(rows, EDITS, b"e%d" % k, rnd), 600)

    def get(rnd, k):
        s.req("GET", "/db/t/get?key=t/big/p%019d" % rnd.randrange(rows), None, 600)

    def search(rnd, k):
        s.req("GET", "/db/t/search?q=word%d&prefix=t/big/&k=10" % rnd.randrange(5000), None, 600)

    ts = [threading.Thread(target=loop, args=a) for a in
          [("append", app), ("edit", edit), ("get", get), ("search", search, 1.0)]]
    curve = []
    t2 = time.time()
    for t in ts:
        t.start()
    while time.time() < stop:
        time.sleep(10)
        curve.append([round(time.time() - t2), nxt[0], rss(s.proc.pid)])
    for t in ts:
        t.join()
    rss1 = rss(s.proc.pid)
    s.stop()
    s.start()
    count = json.loads(s.req("GET", "/db/t/count?prefix=t/big/", None, 1800))["count"]
    s.stop()
    f = subprocess.run([binary, "--store", store, "fsck"], capture_output=True, text=True)
    out = {"flags": flags, "rows": rows, "mix_s": mix, "load_s": round(load_s, 1),
           "warm_search_s": round(warm_s, 2), "rss_start_mb": rss0, "rss_end_mb": rss1,
           "curve": curve, "rows_after": count, "rows_expected": nxt[0],
           "fsck_rc": f.returncode, "fsck_out": f.stdout.strip()[-200:],
           "store_bytes": d.du(store), "fails": fails}
    for k, v in lat.items():
        out[k] = summary(v)
    shutil.rmtree(w, ignore_errors=True)
    return out


def main():
    binary, work, rows, mix, sets = (sys.argv[1], sys.argv[2], int(sys.argv[3]),
                                     int(sys.argv[4]), sys.argv[5].split(";"))
    for fl in sets:
        print(json.dumps(one(binary, work, rows, mix, fl)), flush=True)


if __name__ == "__main__":
    main()
