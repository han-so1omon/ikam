"""E383: does bounding the seed index (`--seed-index-max N`) keep a loading
`ikam db serve` from growing, and what does it cost in store bytes and time?
Loads ROWS indexed rows (E354's rows) in 2,000-row batches, records resident
memory (VmRSS, Linux /proc) every CHECK rows, then the store's bytes and a full
count after a restart. Synthetic rows; 127.0.0.1.
  python3 -I load_mem.py IKAM_BIN WORK ROWS CHECK "FLAGS;FLAGS" """
import json, os, shutil, sys, time
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "goal-scorecard"))
import db_first_write as d

binary, work, rows, check, sets = (sys.argv[1], sys.argv[2], int(sys.argv[3]),
                                   int(sys.argv[4]), sys.argv[5].split(";"))
BATCH = 2000


def rss(pid):
    for line in open("/proc/%d/status" % pid):
        if line.startswith("VmRSS"):
            return round(int(line.split()[1]) / 1024)


for fl in sets:
    w = os.path.join(work, "m"); shutil.rmtree(w, ignore_errors=True); os.makedirs(w)
    store = os.path.join(w, "store")
    s = d.Server(binary, store, fl.split()); s.start()
    curve, t0 = [], time.time()
    for start in range(0, rows, BATCH):
        assert s.append(d.events(b"big", start, min(BATCH, rows - start), b"load"), 1800)[1]
        if (start + BATCH) % check == 0:
            curve.append([start + BATCH, round(time.time() - t0, 1), rss(s.proc.pid)])
    s.req("GET", "/db/t/count?prefix=t/big/", None, 1800)
    loaded = rss(s.proc.pid)
    s.stop(); s.start()
    count = s.req("GET", "/db/t/count?prefix=t/big/", None, 1800).decode()
    s.stop()
    print(json.dumps({"flags": fl, "rows": rows, "curve": curve, "loaded_mb": loaded,
                      "count_after_restart": count, "store_bytes": d.du(store)}), flush=True)
    shutil.rmtree(w, ignore_errors=True)
