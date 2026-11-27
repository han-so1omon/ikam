#!/usr/bin/env python3
"""E394: does a first search over a large table stall appends, and does
`db serve --build-off-lock` keep them going?

Loads ROWS indexed rows (E354's rows, 2,000-row batches), restarts the
server (no view held), then runs a writer sending 2,000-row batches and a
head reader (`GET head` every 0.2 s) for PRE seconds, sends the table's
first ranked search (it builds the BM25 view), and keeps both running for
POST seconds after it answers. Prints append and head latencies before,
during and after the build, the search's time and answer, and the
server's peak memory (VmHWM).

    python3 -I off_lock.py IKAM_BIN WORK ROWS PRE POST "FLAGS;FLAGS"

Synthetic rows; 127.0.0.1 only.
"""
import hashlib, json, os, shutil, sys, threading, time
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "goal-scorecard"))
import db_first_write as d

BATCH = 2000
Q = "/db/t/search?q=word7+word11&prefix=t/big/&k=10"


def status(pid, key):
    for line in open("/proc/%d/status" % pid):
        if line.startswith(key):
            return round(int(line.split()[1]) / 1024)


def pct(xs, p):
    xs = sorted(xs)
    return round(xs[min(len(xs) - 1, int(p * len(xs)))] * 1000, 1) if xs else None


def phase(samples, lo, hi):
    xs = [dt for (t, dt) in samples if lo <= t < hi]
    return {"n": len(xs), "p50_ms": pct(xs, 0.5), "p99_ms": pct(xs, 0.99), "max_ms": pct(xs, 1.0)}


def one(binary, work, rows, pre, post, flags):
    w = os.path.join(work, "m")
    shutil.rmtree(w, ignore_errors=True)
    os.makedirs(w)
    s = d.Server(binary, os.path.join(w, "store"), flags.split())
    s.start()
    for start in range(0, rows, BATCH):
        n = min(BATCH, rows - start)
        assert s.append(d.events(b"big", start, n, b"load"), 1800)[1]
    s.stop()
    s.start()
    appends, heads, done = [], [], [False]
    nxt, failed = [rows], []

    def writer():
        # Counts only batches the server accepted; a failure stops the run.
        while not done[0]:
            start = nxt[0]
            t = time.time()
            try:
                s.req("POST", "/db/t/append", d.events(b"big", start, BATCH, b"w"), 1800)
            except Exception as e:
                failed.append(repr(e))
                return
            nxt[0] += BATCH
            appends.append((t, time.time() - t))

    def header():
        while not done[0]:
            t = time.time()
            s.req("GET", "/db/t/head", None, 1800)
            heads.append((t, time.time() - t))
            time.sleep(0.2)

    ts = [threading.Thread(target=writer), threading.Thread(target=header)]
    for t in ts:
        t.start()
    time.sleep(pre)
    t0 = time.time()
    answer = s.req("GET", Q, None, 1800)
    t1 = time.time()
    time.sleep(post)
    done[0] = True
    for t in ts:
        t.join()
    if failed:
        s.stop()
        raise RuntimeError("an append failed: %s" % failed[0])
    again = s.req("GET", Q, None, 1800)
    hwm = status(s.proc.pid, "VmHWM")
    s.stop()
    out = {"flags": flags, "rows": rows, "search_s": round(t1 - t0, 2),
           "answer_sha": hashlib.sha256(answer).hexdigest()[:12],
           "answer_n": len(json.loads(answer)) if answer[:1] == b"[" else None,
           "rows_appended": nxt[0] - rows, "peak_mb": hwm,
           "append_before": phase(appends, 0, t0), "append_during": phase(appends, t0, t1),
           "append_after": phase(appends, t1, 1e12),
           "head_before": phase(heads, 0, t0), "head_during": phase(heads, t0, t1),
           "head_after": phase(heads, t1, 1e12),
           "later_answer_sha": hashlib.sha256(again).hexdigest()[:12]}
    shutil.rmtree(w, ignore_errors=True)
    return out


def main():
    binary, work = sys.argv[1], sys.argv[2]
    rows, pre, post = int(sys.argv[3]), float(sys.argv[4]), float(sys.argv[5])
    for fl in sys.argv[6].split(";"):
        print(json.dumps(one(binary, work, rows, pre, post, fl)), flush=True)


if __name__ == "__main__":
    main()
