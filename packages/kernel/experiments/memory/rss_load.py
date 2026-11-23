"""E375: how much memory does `ikam db serve` hold as a table grows, and
does it give it back? Loads ROWS indexed rows in 2,000-row batches and records
the server's resident memory (VmRSS) and its peak (VmHWM) from /proc at each
CHECK rows, then after a restart and a full count, then after a ranked search
over the rows' text, then after a second restart and the same search (a kept
search view is read back instead of built). Linux only (/proc). Synthetic rows; 127.0.0.1.
  python3 -I rss_load.py IKAM_BIN WORK ROWS CHECK "FLAGS;FLAGS" """
import json, os, shutil, sys, time
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "goal-scorecard"))
import db_first_write as d

binary, work, rows, check, sets = (sys.argv[1], sys.argv[2], int(sys.argv[3]),
                                   int(sys.argv[4]), sys.argv[5].split(";"))
BATCH = 2000


def mem(pid):
    out = {}
    for line in open("/proc/%d/status" % pid):
        k = line.split(":")[0]
        if k in ("VmRSS", "VmHWM"):
            out[k] = round(int(line.split()[1]) / 1024)
    return out


for fl in sets:
    w = os.path.join(work, "m"); shutil.rmtree(w, ignore_errors=True); os.makedirs(w)
    store = os.path.join(w, "store")
    s = d.Server(binary, store, fl.split()); s.start()
    curve, t0, due = [], time.time(), check
    for start in range(0, rows, BATCH):
        assert s.append(d.events(b"big", start, min(BATCH, rows - start), b"load"), 600)[1]
        loaded = min(start + BATCH, rows)
        if loaded >= due or loaded == rows:  # any CHECK, and the last batch
            curve.append(dict(rows=loaded, s=round(time.time() - t0), **mem(s.proc.pid)))
            while due <= loaded:
                due += check
    s.req("GET", "/db/t/count?prefix=t/big/", None, 600)
    end = mem(s.proc.pid)
    s.stop(); s.start()
    s.req("GET", "/db/t/count?prefix=t/big/", None, 600)
    restarted = mem(s.proc.pid)
    t1 = time.time()
    s.req("GET", "/db/t/search?prefix=t/big/&q=word42&k=10", None, 1800)
    searched = dict(first_search_s=round(time.time() - t1, 1), **mem(s.proc.pid))
    s.stop(); s.start()
    t2 = time.time()
    s.req("GET", "/db/t/search?prefix=t/big/&q=word42&k=10", None, 1800)
    again = dict(search_s=round(time.time() - t2, 1), **mem(s.proc.pid))
    s.stop()
    print(json.dumps({"flags": fl, "rows": rows, "curve": curve, "loaded": end,
                      "restarted": restarted, "searched": searched, "searched_after_restart": again,
                      "store_mb": round(d.du(store) / 1e6)}), flush=True)
    shutil.rmtree(w, ignore_errors=True)
