"""E374: does `ikam db serve` keep every append it answered when the process
is killed with no warning (SIGKILL: an out-of-memory kill, a power cut of the
process), and how does it come back?

For each flag set and TRIALS times: load BASE rows, then a writer sends
2,000-row batches (each with its own idempotency keys and a marker row) while
the server is killed after a random delay. The server is started again on the
same store and checked:
  - every batch the server answered 200 is all there (marker and row count),
  - a batch sent but not answered is all there or not there at all,
  - the store passes `ikam fsck`,
and the restart time and the first read after it are timed.
Synthetic rows, 127.0.0.1 only. Usage:
  python3 -I kill_mid_append.py IKAM_BIN WORK BASE TRIALS "FLAGS;FLAGS;..." """
import json, os, random, shutil, signal, subprocess, sys, threading, time
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "goal-scorecard"))
import db_first_write as d

binary, work, base, trials, sets = (sys.argv[1], sys.argv[2], int(sys.argv[3]),
                                    int(sys.argv[4]), sys.argv[5].split(";"))
BATCH = 2000


def batch_events(n):
    # rows big/p(base + n*BATCH ...) plus a marker row m/<n>
    body = d.events(b"big", base + n * BATCH, BATCH, b"w%d" % n)
    return body + b"\x00" + d.pack(b"m/%08d" % n, b"mark-%d" % n, b"%d" % n)


def count(s, prefix):
    return int(json.loads(s.req("GET", "/db/t/count?prefix=" + prefix, None, 600))["count"])


def present(s, n):
    try:
        s.req("GET", "/db/t/get?key=m/%08d" % n, None, 600)
        return True
    except Exception:
        return False


def trial(flags, k):
    w = os.path.join(work, "t%d" % k)
    shutil.rmtree(w, ignore_errors=True); os.makedirs(w)
    store = os.path.join(w, "store")
    s = d.Server(binary, store, flags)
    s.start()
    for start in range(0, base, BATCH):
        assert s.append(d.events(b"big", start, min(BATCH, base - start), b"load"), 600)[1]
    acked, sent, stop = [], [], threading.Event()

    def writer():
        n = 0
        while not stop.is_set():
            sent.append(n)
            try:
                s.req("POST", "/db/t/append", batch_events(n), 600)
                acked.append(n)
            except Exception:
                return
            n += 1
    th = threading.Thread(target=writer); th.start()
    time.sleep(random.Random(k).uniform(0.5, 6.0))
    s.proc.send_signal(signal.SIGKILL); s.proc.wait(); s.log.close()
    stop.set(); th.join()
    fsck = subprocess.run([binary, "--store", store, "fsck"], capture_output=True, text=True)
    restart = s.start()
    r0 = time.time(); count(s, "m/"); first_read = time.time() - r0
    lost = [n for n in acked if not present(s, n)]
    pending = [n for n in sent if n not in acked]
    landed = [n for n in pending if present(s, n)]
    probe = range(0, BATCH, 499)
    torn = [n for n in landed if sum(1 for i in probe if s.req(
        "GET", "/db/t/get?key=t/big/p%019d" % (base + n * BATCH + i), None, 600)) != len(probe)]
    rows = count(s, "t/big/")
    want = base + BATCH * (len(acked) + len(landed))
    after = s.append(batch_events(10_000), 600)[1]
    s.stop()
    fsck2 = subprocess.run([binary, "--store", store, "fsck"], capture_output=True, text=True)
    shutil.rmtree(w, ignore_errors=True)
    return {"acked": len(acked), "lost": len(lost), "pending": len(pending),
            "pending_landed": len(landed),
            "torn": len(torn), "rows_ok": rows == want, "fsck_after_kill": fsck.returncode == 0,
            "fsck_msg": (fsck.stdout + fsck.stderr).strip()[-200:] if fsck.returncode else "",
            "restart_s": round(restart, 2), "first_read_ms": round(first_read * 1000, 1),
            "append_after": after, "fsck_end": fsck2.returncode == 0}


for fl in sets:
    flags = fl.split()
    res = [trial(flags, k) for k in range(trials)]
    print(json.dumps({"flags": fl, "trials": trials,
                      "acked": sum(r["acked"] for r in res), "lost": sum(r["lost"] for r in res),
                      "torn": sum(r["torn"] for r in res),
                      "pending": sum(r["pending"] for r in res),
                      "pending_landed": sum(r["pending_landed"] for r in res),
                      "rows_ok": sum(r["rows_ok"] for r in res),
                      "fsck_after_kill_ok": sum(r["fsck_after_kill"] for r in res),
                      "fsck_end_ok": sum(r["fsck_end"] for r in res),
                      "append_after_ok": sum(bool(r["append_after"]) for r in res),
                      "restart_s_max": max(r["restart_s"] for r in res),
                      "first_read_ms_max": max(r["first_read_ms"] for r in res),
                      "fsck_msgs": sorted({r["fsck_msg"] for r in res if r["fsck_msg"]})[:3]}),
          flush=True)
