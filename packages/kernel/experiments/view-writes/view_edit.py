"""E381: what a served index view's update after a change costs, and how
much of it is planning the rewritten chunks. Copies STORE (made by the E379
harness, ab_bench.py ... view) and serves it with VIEWS and FLAGS, then runs
100 rounds of (100 row changes; a point read, which folds them; an index
lookup, which brings the view to the head) and records each step's p50 and
how much the store grew. MODE spread changes rows from 100 different load
batches a round, contiguous 100 neighbouring rows. Synthetic rows; 127.0.0.1.
  python3 -I view_edit.py IKAM_BIN STORE VIEWS spread|contiguous "FLAGS" """
import base64, http.client, json, os, shutil, statistics, struct, subprocess, sys, time
b, src, views = sys.argv[1], sys.argv[2], sys.argv[3]
PW = "p"
auth = subprocess.run([b, "passwd", "u"], input=PW + "\n", capture_output=True, text=True, check=True).stdout.strip().split("=", 1)[1]
h = {"Authorization": "Basic " + base64.b64encode(("u:" + PW).encode()).decode()}
pack = lambda *ps: b"".join(struct.pack(">I", len(p)) + p for p in ps)
SPREAD = sys.argv[4] == "spread"
for flags in (sys.argv[5].split(),):
    store = src + ".mix"; shutil.rmtree(store, ignore_errors=True); shutil.copytree(src, store)
    du = lambda: sum(os.path.getsize(os.path.join(dp, f)) for dp, _, fs in os.walk(store) for f in fs)
    before = du()
    p = subprocess.Popen([b, "--store", store, "db", "serve", *flags, "--views", views, "18556"],
                         env=dict(os.environ, IKAM_AUTH=auth), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(300):
        try:
            c = http.client.HTTPConnection("127.0.0.1", 18556, timeout=600); c.request("GET", "/db/t/head", headers=h); c.getresponse().read(); break
        except OSError:
            time.sleep(0.1)
    def call(m, path, body=None):
        hh = dict(h)
        if body is not None: hh["Content-Type"] = "application/x-ikam-events"
        c.request(m, path, body=body, headers=hh); r = c.getresponse(); d = r.read(); assert r.status == 200, (r.status, d[:200]); return d
    t = time.perf_counter(); call("GET", "/db/t/view?name=by_word&keys=1&eq=word1")
    first = time.perf_counter() - t
    t0, w, rd, fo, seq = time.time(), [], [], [], 0
    for rnd in range(100):
        out = []
        for i in range(100):
            seq += 1; pk = "p%019d" % (((rnd * 997 + i * 1999) if SPREAD else (rnd * 100 + i)) % 200000)
            val = json.dumps({"date": "2026-01-01", "word": "word%d" % ((seq * 7) % 5000), "num": seq, "pad": "x" * 40}).encode()
            out.append(b"\x00" + pack(("t/r/" + pk).encode(), b"m%d" % seq, val))
        t = time.perf_counter(); call("POST", "/db/t/append", b"".join(out)); w.append(time.perf_counter() - t)
        t = time.perf_counter(); call("GET", "/db/t/get?key=t/r/p%019d" % rnd); fo.append(time.perf_counter() - t)
        t = time.perf_counter(); call("GET", "/db/t/view?name=by_word&keys=1&eq=word%d" % rnd); rd.append(time.perf_counter() - t)
    total = time.time() - t0
    p.terminate(); p.wait()
    print(json.dumps({"flags": " ".join(flags), "first_lookup_ms": round(first * 1e3, 1),
                      "total_s": round(total, 1),
                      "append_p50_ms": round(statistics.median(w) * 1e3, 1),
                      "fold_p50_ms": round(statistics.median(fo) * 1e3, 1), "lookup_p50_ms": round(statistics.median(rd) * 1e3, 1),
                      "lookup_max_ms": round(max(rd) * 1e3, 1), "mode": sys.argv[4], "grew_mb": round((du() - before) / 1e6, 1)}), flush=True)
    shutil.rmtree(store, ignore_errors=True)
