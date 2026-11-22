"""E368: load ROWS through `ikam db serve` in 2,000-row batches with each flag
set, reading one just-written row after every READ_EVERY batches (a read of
the head, which folds a deferred view first). Prints the load time, batch and
read percentiles and the store size per set. Usage:
defer_kv_served.py IKAM_BIN WORK ROWS READ_EVERY "FLAGS;FLAGS;..." """
import json, os, shutil, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import db_first_write as d

binary, work, rows, every, sets = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4]), sys.argv[5].split(";")
batch = 2000

def pct(xs, p):
    xs = sorted(xs)
    return round(xs[min(len(xs) - 1, int(p * len(xs)))] * 1000, 1)

for k, fl in enumerate(sets):
    w = os.path.join(work, "set%d" % k)
    shutil.rmtree(w, ignore_errors=True); os.makedirs(w)
    s = d.Server(binary, os.path.join(w, "store"), fl.split())
    s.start()
    appends, reads, t0 = [], [], time.time()
    for n, start in enumerate(range(0, rows, batch)):
        t, ok = s.append(d.events(b"big", start, min(batch, rows - start), b"load"), timeout=600)
        assert ok
        appends.append(t)
        if (n + 1) % every == 0:
            key = "t/big/p%019d" % (start + batch - 1)
            r0 = time.time()
            assert s.req("GET", "/db/t/get?key=" + key, None, 600)
            reads.append(time.time() - r0)
    load = time.time() - t0
    r0 = time.time(); s.req("GET", "/db/t/count?prefix=t/big/", None, 600); last = time.time() - r0
    s.stop()
    print(json.dumps({"flags": fl, "load_s": round(load, 1), "append_p50_ms": pct(appends, 0.5),
                      "append_p99_ms": pct(appends, 0.99), "read_p50_ms": pct(reads, 0.5),
                      "read_max_ms": pct(reads, 1.0), "count_ms": round(last * 1000, 1),
                      "store_mb": round(d.du(os.path.join(w, "store")) / 1e6, 1)}), flush=True)
    shutil.rmtree(w, ignore_errors=True)
