"""E376: what does sending a table again cost when almost nothing changed
(a collector that polls its source and writes every row it gets back)?

Loads ROWS indexed rows through `ikam db serve` in 2,000-row batches, then
sends the same rows twice more with new idempotency keys: once unchanged and
once with CHANGED_PCT of rows given a new value. Times each pass and reports
the store's growth and the log's length after it. Synthetic rows; 127.0.0.1.
  python3 -I reimport_unchanged.py IKAM_BIN WORK ROWS CHANGED_PCT "FLAGS;FLAGS" """
import json, os, shutil, sys, time
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "goal-scorecard"))
import db_first_write as d

binary, work, rows, pct, sets = (sys.argv[1], sys.argv[2], int(sys.argv[3]),
                                 float(sys.argv[4]), sys.argv[5].split(";"))
BATCH = 2000


def changed(body, start, n, every):
    # rewrite one row in `every` with a new value (and its index entry)
    if every <= 0:
        return body
    extra = []
    for i in range(start, start + n):
        if i % every == 0:
            pk = b"p%019d" % i
            val = json.dumps([i, "changed", "word%d" % (i % 5000), 1.0, "y" * 60]).encode()
            extra.append(b"\x00" + d.pack(b"t/big/" + pk, b"chg-%d" % i, val))
    return body + b"".join(extra)


def head(s):
    return json.loads(s.req("GET", "/db/t/head", None, 600))


for fl in sets:
    w = os.path.join(work, "r"); shutil.rmtree(w, ignore_errors=True); os.makedirs(w)
    store = os.path.join(w, "store")
    s = d.Server(binary, store, fl.split()); s.start()
    out = {"flags": fl, "rows": rows}
    every = int(round(100 / pct)) if pct > 0 else 0
    for name, run, ev in (("load", b"load", 0), ("same", b"again", 0), ("changed", b"third", every)):
        t0 = time.time(); lat = []
        for start in range(0, rows, BATCH):
            n = min(BATCH, rows - start)
            t, ok = s.append(changed(d.events(b"big", start, n, run), start, n, ev), 600)
            assert ok, name
            lat.append(t)
        s.req("GET", "/db/t/count?prefix=t/big/", None, 600)  # settles a deferred view
        lat.sort()
        out[name] = {"s": round(time.time() - t0, 1), "batch_p50_ms": round(lat[len(lat) // 2] * 1000, 1),
                     "store_mb": round(d.du(store) / 1e6, 1), "log": head(s).get("head")}
    s.stop()
    print(json.dumps(out), flush=True)
    shutil.rmtree(w, ignore_errors=True)
