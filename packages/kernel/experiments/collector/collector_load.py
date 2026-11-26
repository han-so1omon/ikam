"""E412: a collector's database under `ikam db serve`.

A collector is a set of scheduled jobs that each poll a source every 15
minutes and write what they saw: one observation row per item (its history),
an index entry by time, and the item's latest row. Most items look the same
as last time except for when they were seen; a few change price or status,
and a few come and go. Once an hour a reader asks what changed: every
item's latest row for one job, and the last hour's observations by time.

Time is simulated (no waiting between polls), so DAYS of polls run as fast
as the server takes them. Reports, per simulated day: append latency of a
job's batch, the hourly reader's latency (asked straight after a write, so
it is read-after-write), whether the reader saw every row just written, the
store's size and the server's resident memory. After the last day the
server is stopped and `ikam repack` run on the store.

Synthetic rows; 127.0.0.1 only.
  python3 -I collector_load.py IKAM_BIN WORK DAYS JOBS ITEMS "FLAGS;FLAGS" [per-job|per-tick] [KEEP]
per-job (the default) sends one append per job per poll; per-tick sends all
jobs' rows of a poll as one append. KEEP keeps the last store in WORK.
COLLECTOR_GAP=S in the environment waits S seconds of real time after each
poll's writes (before the hourly reader, when it runs), as the quiet
between a real collector's polls.
"""
import json, os, random, shutil, subprocess, sys, time
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "goal-scorecard"))
import db_first_write as d

binary, work = sys.argv[1], sys.argv[2]
days, jobs, items = int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5])
sets = sys.argv[6].split(";")
mode = sys.argv[7] if len(sys.argv) > 7 else "per-job"
keep = len(sys.argv) > 8 and sys.argv[8] == "KEEP"
gap = float(os.environ.get("COLLECTOR_GAP", "0"))
TICK = 900
START = 1_795_000_000  # a fixed epoch second, so runs are comparable
WORDS = ("blue steel oak linen small large classic travel kit pack set "
         "case lamp chair desk shelf mug bowl print poster cable charger").split()


def iso(ts):
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(ts))


class Job:
    """One source: ITEMS items, each with a price, stock, status and title."""

    def __init__(self, j):
        self.j, self.rnd, self.next_id = j, random.Random(412 + j), 0
        self.items, self.gone = {}, []
        for _ in range(items):
            self.add()

    def add(self):
        r = self.rnd
        self.items["i%07d" % self.next_id] = {
            "price": round(r.uniform(2, 400), 2), "qty": r.randrange(0, 50),
            "status": "in_stock",
            "title": " ".join(r.choice(WORDS) for _ in range(r.randrange(4, 14))),
        }
        self.next_id += 1

    def poll(self):
        """Advance the source by one poll: a few items change or turn over."""
        r = self.rnd
        for k in list(self.items):
            x = r.random()
            if x < 0.01:
                del self.items[k]
                self.gone.append(k)
                self.add()
            elif x < 0.06:
                self.items[k]["price"] = round(self.items[k]["price"] * r.uniform(0.9, 1.1), 2)
            elif x < 0.08:
                self.items[k]["qty"] = r.randrange(0, 50)
                self.items[k]["status"] = "in_stock" if self.items[k]["qty"] else "sold_out"

    def events(self, tick, ts):
        out, raw = [], 0
        for k, it in self.items.items():
            val = json.dumps({"item": k, "job": "j%02d" % self.j, "observed_at": iso(ts),
                              "price": it["price"], "qty": it["qty"], "status": it["status"],
                              "title": it["title"],
                              "url": "https://example.com/j%02d/%s" % (self.j, k)},
                             separators=(",", ":")).encode()
            pk = b"t/obs/j%02d/%s/%010d" % (self.j, k.encode(), ts)
            idem = b"j%02d-%d-%s" % (self.j, tick, k.encode())
            out.append(b"\x00" + d.pack(pk, idem + b"-o", val))
            out.append(b"\x00" + d.pack(b"i/obs/by_time/%010d/j%02d/%s" % (ts, self.j, k.encode()),
                                        idem + b"-i", pk))
            out.append(b"\x00" + d.pack(b"t/latest/j%02d/%s" % (self.j, k.encode()),
                                        idem + b"-l", val))
            raw += 2 * len(val) + len(pk) * 2 + 60
        for k in self.gone:  # an item the source no longer lists loses its latest row
            out.append(b"\x01" + d.pack(b"t/latest/j%02d/%s" % (self.j, k.encode()),
                                        b"j%02d-%d-%s-r" % (self.j, tick, k.encode())))
        self.gone = []
        return out, raw


def rss_mb(pid):
    try:
        with open("/proc/%d/status" % pid) as f:
            for line in f:
                if line.startswith("VmRSS:"):
                    return round(int(line.split()[1]) / 1024)
    except OSError:
        return None


def pct(xs, p):
    xs = sorted(xs)
    return round(xs[min(len(xs) - 1, int(len(xs) * p))] * 1000, 1) if xs else None


for fl in sets:
    w = os.path.join(work, "c"); shutil.rmtree(w, ignore_errors=True); os.makedirs(w)
    store = os.path.join(w, "store")
    s = d.Server(binary, store, fl.split()); s.start()
    src = [Job(j) for j in range(jobs)]
    out = {"flags": fl, "mode": mode, "jobs": jobs, "items": items, "gap_s": gap, "days": []}
    raw_total, t_all = 0, time.time()
    for day in range(days):
        app, rd_latest, rd_hour, rd_scan, missing, t_day = [], [], [], [], 0, time.time()
        for t in range(96):
            tick = day * 96 + t
            ts = START + tick * TICK
            batches = []
            for job in src:
                job.poll()
                ev, raw = job.events(tick, ts)
                raw_total += raw
                batches.append(ev)
            if mode == "per-tick":
                batches = [[e for b in batches for e in b]]
            for b in batches:
                dt, ok = s.append(b"".join(b), 600)
                assert ok, (fl, tick)
                app.append(dt)
            if gap:
                time.sleep(gap)
            if tick % 4 == 3:  # the hourly reader, straight after the writes
                jn = (tick // 4) % jobs
                t0 = time.time()
                got = s.req("GET", "/db/t/scan?prefix=t/latest/j%02d/&format=json" % jn, None, 600)
                rd_latest.append(time.time() - t0)
                newest = json.loads(got)
                rows = newest if isinstance(newest, list) else newest.get("pairs", newest)
                t0 = time.time()
                q = "/db/t/count?start=i/obs/by_time/%010d&end=i/obs/by_time/%010d" % (
                    ts - 3 * TICK, ts + 1)
                n = json.loads(s.req("GET", q, None, 600))["count"]
                rd_hour.append(time.time() - t0)
                missing += 4 * jobs * items - n
                t0 = time.time()
                s.req("GET", "/db/t/scan?start=i/obs/by_time/%010d&end=i/obs/by_time/%010d&limit=500"
                      % (ts - 3 * TICK, ts + 1), None, 600)
                rd_scan.append(time.time() - t0)
                if len(rows) != items or iso(ts) not in got.decode():
                    missing += 1
        out["days"].append({
            "day": day + 1, "wall_s": round(time.time() - t_day, 1),
            "append_p50_ms": pct(app, 0.5), "append_p95_ms": pct(app, 0.95),
            "append_max_ms": pct(app, 1.0),
            "latest_p50_ms": pct(rd_latest, 0.5), "latest_max_ms": pct(rd_latest, 1.0),
            "hour_count_p50_ms": pct(rd_hour, 0.5), "hour_count_max_ms": pct(rd_hour, 1.0),
            "hour_scan_p50_ms": pct(rd_scan, 0.5), "hour_scan_max_ms": pct(rd_scan, 1.0),
            "reader_missed": missing, "store_mb": round(d.du(store) / 1e6, 1),
            "rss_mb": rss_mb(s.proc.pid)})
        print(json.dumps(out["days"][-1]), flush=True)
    out["total_s"] = round(time.time() - t_all, 1)
    out["observations"] = days * 96 * jobs * items
    out["raw_mb"] = round(raw_total / 1e6, 1)
    s.stop()
    out["store_mb"] = round(d.du(store) / 1e6, 1)
    t0 = time.time()
    r = subprocess.run([binary, "--store", store, "repack"], capture_output=True, text=True)
    out["repack_s"] = round(time.time() - t0, 1)
    out["repack_ok"] = r.returncode == 0
    out["repack_out"] = (r.stdout + r.stderr).strip()[-200:]
    out["after_repack_mb"] = round(d.du(store) / 1e6, 1)
    out["bytes_per_obs_after_repack"] = round(d.du(store) / out["observations"], 1)
    print(json.dumps(out), flush=True)
    if not keep:
        shutil.rmtree(w, ignore_errors=True)
