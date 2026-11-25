#!/usr/bin/env python3
"""E392: memory peak of a deferred kv catch-up. Loads ROWS rows with
--defer-kv set past the load (no fold during it), then one count makes the
server fold every waiting event. Prints VmHWM before and after, the count's
time, the head and the store's bytes.

    python3 -I fold_peak.py IKAM_BIN WORK ROWS "FLAGS"

Synthetic rows (E354's); 127.0.0.1 only."""
import json, os, shutil, sys, time
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "goal-scorecard"))
import db_first_write as d
binary, work, rows = sys.argv[1], sys.argv[2], int(sys.argv[3])
flags = sys.argv[4].split()
def status(pid, key):
    for line in open("/proc/%d/status" % pid):
        if line.startswith(key):
            return round(int(line.split()[1]) / 1024)
shutil.rmtree(work, ignore_errors=True); os.makedirs(work)
store = os.path.join(work, "store")
s = d.Server(binary, store, flags); s.start()
for start in range(0, rows, 2000):
    assert s.append(d.events(b"big", start, min(2000, rows - start), b"load"), 1800)[1]
hw0, rss0 = status(s.proc.pid, "VmHWM"), status(s.proc.pid, "VmRSS")
t0 = time.time()
c = s.req("GET", "/db/t/count?prefix=t/big/", None, 1800).decode()
dt = time.time() - t0
hw1, rss1 = status(s.proc.pid, "VmHWM"), status(s.proc.pid, "VmRSS")
head = s.req("GET", "/db/t/head", None, 60).decode()
s.stop()
print(json.dumps({"bin": os.path.basename(binary), "flags": " ".join(flags), "rows": rows,
  "hwm_before_mb": hw0, "rss_before_mb": rss0, "hwm_after_mb": hw1, "rss_after_mb": rss1,
  "catch_up_s": round(dt, 2), "count": c, "head": head, "store_bytes": d.du(store)}), flush=True)
