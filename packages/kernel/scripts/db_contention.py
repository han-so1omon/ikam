#!/usr/bin/env python3
"""E232: reads and appends against `ikam db serve` at the same time.

Readers loop over two pages' worth of requests on a preloaded database
(`examples/db_write_path.rs DIR ROWS 0` writes one, database `bench`):
a span over every group (as a list view asks for counts and ends) and one
group's whole range. Writers send small appends on a schedule. Prints
append and read times, idle and under load. Standard library only.

  python3 scripts/db_contention.py URL [--readers 4] [--writers 1] [--appends 200]
      [--every 0.05] [--per 1] [--seconds-idle 3]
"""

import argparse
import json
import statistics
import struct
import threading
import time
import urllib.error
import urllib.request


def field(b: bytes) -> bytes:
    return struct.pack(">I", len(b)) + b


def events(rows):
    return b"".join(b"\x00" + field(k.encode()) + field(i.encode()) + field(v) for k, i, v in rows)


def call(url, method="GET", body=None, headers=None):
    req = urllib.request.Request(url, data=body, method=method, headers=headers or {})
    try:
        with urllib.request.urlopen(req, timeout=300) as r:
            return r.read()
    except urllib.error.HTTPError as e:
        raise SystemExit(f"{method} {url}: {e.code} {e.read()[:300]!r}") from None


def pct(xs):
    if not xs:
        return None
    xs = sorted(xs)
    return {"n": len(xs), "p50": round(statistics.median(xs), 2),
            "p95": round(xs[int(0.95 * (len(xs) - 1))], 2), "max": round(xs[-1], 2)}


def main():
    a = argparse.ArgumentParser()
    a.add_argument("url", help="e.g. http://127.0.0.1:8791")
    a.add_argument("--db", default="bench")
    a.add_argument("--readers", type=int, default=4)
    a.add_argument("--writers", type=int, default=1)
    a.add_argument("--appends", type=int, default=200, help="per writer")
    a.add_argument("--every", type=float, default=0.05, help="seconds between a writer's appends")
    a.add_argument("--per", type=int, default=1, help="rows per append")
    a.add_argument("--groups", type=int, default=200)
    a.add_argument("--seconds-idle", type=float, default=3, help="reads alone first, and appends alone")
    args = a.parse_args()
    base = f"{args.url.rstrip('/')}/db/{args.db}/"
    span_body = "".join(f"t/rows/G{s:04}/\n" for s in range(args.groups)).encode()
    reads = [("span", lambda: call(base + "span", "POST", span_body)),
             ("scan", lambda: call(base + "scan?prefix=t/rows/G0003/"))]

    def append(w, i):
        rows = [(f"t/rows/W{w:03}/{i:06}-{j:03}", f"bench:{run}:{w}:{i}:{j}",
                 b'["W",1.25,2.5,"bench","2026-01-05T21:00:00+00:00"]') for j in range(args.per)]
        t = time.perf_counter()
        call(base + "append", "POST", events(rows), {"Content-Type": "application/x-ikam-events"})
        return (time.perf_counter() - t) * 1000

    run = time.time_ns()
    out = {"readers": args.readers, "writers": args.writers, "per": args.per}
    # Appends alone.
    alone = [append(999, i) for i in range(max(10, int(args.seconds_idle / args.every)))]
    out["append_ms_alone"] = pct(alone)
    # Reads alone (one reader).
    samples = {n: [] for n, _ in reads}
    t_end = time.perf_counter() + args.seconds_idle
    while time.perf_counter() < t_end:
        for n, f in reads:
            t = time.perf_counter()
            f()
            samples[n].append((time.perf_counter() - t) * 1000)
    out["read_ms_alone"] = {n: pct(v) for n, v in samples.items()}

    stop = threading.Event()
    rsamples = {n: [] for n, _ in reads}
    lock = threading.Lock()

    def reader():
        while not stop.is_set():
            for n, f in reads:
                t = time.perf_counter()
                f()
                with lock:
                    rsamples[n].append((time.perf_counter() - t) * 1000)

    wl = []

    def writer(w):
        t0 = time.perf_counter()
        for i in range(args.appends):
            wait = t0 + i * args.every - time.perf_counter()
            if wait > 0:
                time.sleep(wait)
            ms = append(w, i)
            with lock:
                wl.append(ms)

    rs = [threading.Thread(target=reader, daemon=True) for _ in range(args.readers)]
    for t in rs:
        t.start()
    time.sleep(0.5)
    t0 = time.perf_counter()
    ws = [threading.Thread(target=writer, args=(w,)) for w in range(args.writers)]
    for t in ws:
        t.start()
    for t in ws:
        t.join()
    took = time.perf_counter() - t0
    stop.set()
    for t in rs:
        t.join(300)
    out["append_ms_with_readers"] = pct(wl)
    out["appends_per_s"] = round(len(wl) / took, 1)
    out["read_ms_with_writers"] = {n: pct(v) for n, v in rsamples.items()}
    out["reads_per_s"] = round(sum(len(v) for v in rsamples.values()) / took, 1)
    print(json.dumps(out, indent=1))


if __name__ == "__main__":
    main()
