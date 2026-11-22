#!/usr/bin/env python3
"""E361: kept views over `ikam db serve` against grouping in the client.

Loads ROWS synthetic orders (JSON with a customer, a status, an amount and a
day) into `db serve --live --views FILE`, then answers three questions the
way a client must today (scan the table, parse, group or filter in its own
code) and through the kept views: totals per status, orders per day, and
one customer's orders. Then rewrites, adds and removes some orders and asks
again; then restarts. Checks that both ways agree every time.

    python3 -I served_views.py IKAM_BIN WORK_DIR [ROWS] [EXTRA_FLAGS...]

Prints JSON lines. Synthetic data only; 127.0.0.1 only.
"""
import json
import os
import random
import shutil
import statistics
import struct
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "goal-scorecard"))
import db_first_write as d  # noqa: E402

VIEWS = """by_status group t/orders/ by=field:status sum=field:amount
per_day group t/orders/ by=field:day
per_customer group t/orders/ by=field:customer
orders_of index t/orders/ by=field:customer
"""
STATUSES = ["new", "paid", "packed", "shipped", "returned"]
BATCH = 2000


def order(rnd, i):
    return {
        "customer": "c%05d" % rnd.randrange(max(1, ROWS // 20)),
        "status": rnd.choice(STATUSES),
        "amount": rnd.randrange(100, 50000),
        "day": "2026-%02d-%02d" % (1 + i % 12, 1 + i % 28),
        "note": "x" * rnd.randrange(20, 120),
    }


def upsert(key, idem, value):
    return b"\x00" + d.pack(key.encode(), idem.encode(), value)


def remove(key, idem):
    return b"\x01" + d.pack(key.encode(), idem.encode())


def frames(b):
    out, i = [], 0
    while i < len(b):
        (n,) = struct.unpack(">I", b[i:i + 4]); i += 4
        k = b[i:i + n].decode(); i += n
        (n,) = struct.unpack(">I", b[i:i + 4]); i += 4
        out.append((k, b[i:i + n])); i += n
    return out


def get(s, path, timeout=600):
    t0 = time.time()
    body = s.req("GET", path, None, timeout)
    return time.time() - t0, body


def client(s, customer):
    """What a client does today: scan the table, parse and group."""
    t0 = time.time()
    rows = frames(s.req("GET", "/db/t/scan?prefix=t/orders/", None, 600))
    t_scan = time.time() - t0
    by_status, per_day, mine = {}, {}, []
    for k, v in rows:
        o = json.loads(v)
        c, a = by_status.get(o["status"], (0, 0))
        by_status[o["status"]] = (c + 1, a + o["amount"])
        per_day[o["day"]] = per_day.get(o["day"], 0) + 1
        if o["customer"] == customer:
            mine.append(k)
    return {"scan_s": t_scan, "total_s": time.time() - t0, "rows": len(rows),
            "by_status": by_status, "per_day": per_day, "mine": mine}


def served(s, customer):
    t1, b = get(s, "/db/t/view?name=by_status")
    by_status = {k: tuple(int(x) for x in v.split(b",")) for k, v in frames(b)}
    t2, b = get(s, "/db/t/view?name=per_day")
    per_day = {k: int(v.split(b",")[0]) for k, v in frames(b)}
    t3, b = get(s, "/db/t/view?name=orders_of&eq=" + customer)
    mine = [k for k, _ in frames(b)]
    return {"by_status_s": t1, "per_day_s": t2, "orders_of_s": t3,
            "by_status": by_status, "per_day": per_day, "mine": mine}


def agree(a, b):
    return a["by_status"] == b["by_status"] and a["per_day"] == b["per_day"] and a["mine"] == b["mine"]


def rss_mb(pid):
    with open("/proc/%d/status" % pid) as f:
        for line in f:
            if line.startswith("VmRSS"):
                return int(line.split()[1]) / 1024
    return None


def repeat(s, customers, n=30):
    st, pd, oo = [], [], []
    for i in range(n):
        st.append(get(s, "/db/t/view?name=by_status")[0])
        pd.append(get(s, "/db/t/view?name=per_day")[0])
        oo.append(get(s, "/db/t/view?name=orders_of&eq=" + customers[i % len(customers)])[0])
    ms = lambda xs: [round(statistics.median(xs) * 1000, 2), round(max(xs) * 1000, 2)]
    return {"by_status_ms": ms(st), "per_day_ms": ms(pd), "orders_of_ms": ms(oo)}


def main():
    global ROWS
    binary, work = sys.argv[1], sys.argv[2]
    ROWS = int(sys.argv[3]) if len(sys.argv) > 3 else 200000
    extra = sys.argv[4:]
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)
    vf = os.path.join(work, "views.txt")
    open(vf, "w").write(VIEWS)
    store = os.path.join(work, "store")
    s = d.Server(binary, store, ["--live", "--views", vf] + extra)
    s.start()
    rnd = random.Random(361)
    t0 = time.time()
    for start in range(0, ROWS, BATCH):
        body = b"".join(upsert("t/orders/o%08d" % i, "l%d" % i, json.dumps(order(rnd, i)).encode())
                        for i in range(start, min(ROWS, start + BATCH)))
        dt, ok = s.append(body, timeout=600)
        assert ok
    print(json.dumps({"rows": ROWS, "flags": extra, "load_s": round(time.time() - t0, 1),
                      "store_mb": round(d.du(store) / 1e6, 1), "rss_mb": rss_mb(s.proc.pid)}), flush=True)
    customers = ["c%05d" % rnd.randrange(max(1, ROWS // 20)) for _ in range(30)]
    c = client(s, customers[0])
    print(json.dumps({"step": "client", "scan_s": round(c["scan_s"], 3), "total_s": round(c["total_s"], 3),
                      "rows": c["rows"]}), flush=True)
    t0 = time.time()
    first = served(s, customers[0])
    build = time.time() - t0
    print(json.dumps({"step": "first view request (build)", "build_s": round(build, 3),
                      "agree": agree(c, first), "store_mb": round(d.du(store) / 1e6, 1),
                      "rss_mb": rss_mb(s.proc.pid)}), flush=True)
    print(json.dumps({"step": "warm", **repeat(s, customers)}), flush=True)
    # Changes: 1% of the orders rewritten, 1% added, 0.1% removed.
    n = max(1, ROWS // 100)
    body = []
    for j in range(n):
        i = rnd.randrange(ROWS)
        body.append(upsert("t/orders/o%08d" % i, "u%d" % j, json.dumps(order(rnd, i)).encode()))
    for j in range(n):
        i = ROWS + j
        body.append(upsert("t/orders/o%08d" % i, "a%d" % j, json.dumps(order(rnd, i)).encode()))
    for j in range(n // 10):
        body.append(remove("t/orders/o%08d" % rnd.randrange(ROWS), "r%d" % j))
    dt, ok = s.append(b"".join(body), timeout=600)
    t0 = time.time()
    after = served(s, customers[1])
    upd = time.time() - t0
    c2 = client(s, customers[1])
    print(json.dumps({"step": "after %d changes" % len(body), "append_s": round(dt, 3),
                      "update_s": round(upd, 3), "agree": agree(c2, after),
                      "client_total_s": round(c2["total_s"], 3), "store_mb": round(d.du(store) / 1e6, 1)}), flush=True)
    print(json.dumps({"step": "warm after", **repeat(s, customers)}), flush=True)
    # A small change, as one request would make: 0.01% rewritten.
    small = []
    for j in range(max(1, ROWS // 10000)):
        i = rnd.randrange(ROWS)
        small.append(upsert("t/orders/o%08d" % i, "s%d" % j, json.dumps(order(rnd, i)).encode()))
    dt, ok = s.append(b"".join(small), timeout=600)
    t0 = time.time()
    s_after = served(s, customers[3])
    upd = time.time() - t0
    c4 = client(s, customers[3])
    print(json.dumps({"step": "after %d changes" % len(small), "append_s": round(dt, 3),
                      "update_s": round(upd, 3), "agree": agree(c4, s_after)}), flush=True)
    s.stop()
    s.start()
    t0 = time.time()
    again = served(s, customers[2])
    rebuild = time.time() - t0
    c3 = client(s, customers[2])
    print(json.dumps({"step": "after restart", "build_s": round(rebuild, 3), "agree": agree(c3, again),
                      "store_mb": round(d.du(store) / 1e6, 1), "rss_mb": rss_mb(s.proc.pid)}), flush=True)
    s.stop()


if __name__ == "__main__":
    main()
