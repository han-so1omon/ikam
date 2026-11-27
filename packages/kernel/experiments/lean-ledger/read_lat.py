"""E391: point-read latency through db serve, ledger full or lean. Loads ROWS
rows, restarts, times one get (builds the ledger), then N random gets."""
import json, os, random, shutil, sys, time
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "goal-scorecard"))
import db_first_write as d
binary, work, rows, n = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
for fl in sys.argv[5].split(";"):
    shutil.rmtree(work, ignore_errors=True); os.makedirs(work)
    s = d.Server(binary, os.path.join(work, "store"), fl.split()); s.start()
    for start in range(0, rows, 2000):
        assert s.append(d.events(b"big", start, 2000, b"load"), 1800)[1]
    s.stop(); s.start()
    rnd = random.Random(5)
    t = time.time(); s.req("GET", "/db/t/get?key=t/big/p%019d" % 1, None, 600); first = time.time() - t
    lat = []; vals = []
    for _ in range(n):
        k = rnd.randrange(rows); t = time.time()
        v = s.req("GET", "/db/t/get?key=t/big/p%019d" % k, None, 600)
        lat.append(time.time() - t); vals.append(len(v))
    st = open("/proc/%d/status" % s.proc.pid).read()
    rssmb = [int(l.split()[1]) // 1024 for l in st.splitlines() if l.startswith("VmRSS")][0]
    s.stop(); lat.sort()
    print(json.dumps({"flags": fl, "rows": rows, "first_get_s": round(first, 3),
        "p50_ms": round(lat[n // 2] * 1000, 2), "p99_ms": round(lat[int(n * .99)] * 1000, 2),
        "mean_ms": round(sum(lat) / n * 1000, 2), "bytes_read": sum(vals), "rss_mb": rssmb}), flush=True)
