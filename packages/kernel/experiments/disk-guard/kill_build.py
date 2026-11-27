#!/usr/bin/env python3
"""E404: kill `db serve` while a table's first search builds its BM25 view
(kept on a ref by --keep-search, on in --live), restart, and check that the
answer is a full build's: nothing half-built is read.

    python3 -I kill_build.py IKAM_BIN STORE KILL_AFTER_S [ROUNDS]

STORE is a database store with a table under t/r/ (view-writes/make_store.py
makes one). Each round copies it, starts the server, asks a search, kills the
server KILL_AFTER_S into the build, restarts it, asks again, and compares the
hits with those of a copy whose build was never interrupted. Then the store's
refs are listed and `fsck` is run. Prints one JSON line. 127.0.0.1 only.
"""
import base64, http.client, json, os, shutil, signal, subprocess, sys, threading, time

b, src, kill_after = sys.argv[1], sys.argv[2], float(sys.argv[3])
rounds = int(sys.argv[4]) if len(sys.argv) > 4 else 3
PORT, PW = 18560, "p"
auth = subprocess.run([b, "passwd", "u"], input=PW + "\n", capture_output=True, text=True, check=True).stdout.strip().split("=", 1)[1]
H = {"Authorization": "Basic " + base64.b64encode(("u:" + PW).encode()).decode()}
Q = "/db/t/search?q=word17&prefix=t/r/&k=20"


def start(store):
    p = subprocess.Popen([b, "--store", store, "db", "serve", "--live", str(PORT)],
                         env=dict(os.environ, IKAM_AUTH=auth), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(600):
        try:
            c = http.client.HTTPConnection("127.0.0.1", PORT, timeout=600)
            c.request("GET", "/db/t/head", headers=H)
            c.getresponse().read()
            return p
        except OSError:
            time.sleep(0.05)
    raise SystemExit("server did not start")


def search():
    c = http.client.HTTPConnection("127.0.0.1", PORT, timeout=600)
    t = time.perf_counter()
    c.request("GET", Q, headers=H)
    r = c.getresponse()
    body = r.read()
    return r.status, body, time.perf_counter() - t


def refs(store):
    out = []
    for dp, _, fs in os.walk(os.path.join(store, "refs")):
        out += [os.path.relpath(os.path.join(dp, f), store) for f in fs if not f.endswith(".lock")]
    return sorted(out)


# The answer of a build that was never interrupted.
ref_store = src + ".full"
shutil.rmtree(ref_store, ignore_errors=True)
shutil.copytree(src, ref_store)
p = start(ref_store)
status, want, full_s = search()
p.terminate(); p.wait()
res = {"full_build_s": round(full_s, 2), "full_status": status, "refs_after_full": refs(ref_store), "rounds": []}
for i in range(rounds):
    st = src + ".kill"
    shutil.rmtree(st, ignore_errors=True)
    shutil.copytree(src, st)
    p = start(st)
    got = {}
    def ask():
        try:
            got["r"] = search()
        except OSError:
            pass  # the server was killed before it answered

    th = threading.Thread(target=ask, daemon=True)
    th.start()
    time.sleep(kill_after)
    p.send_signal(signal.SIGKILL); p.wait()
    killed_mid = "r" not in got
    left = refs(st)
    p = start(st)
    status, body, s = search()
    p.terminate(); p.wait()
    fsck = subprocess.run([b, "--store", st, "fsck"], capture_output=True, text=True)
    res["rounds"].append({
        "killed_during_build": killed_mid,
        "refs_after_kill": left,
        "status": status,
        "same_hits": body == want,
        "search_after_restart_s": round(s, 2),
        "fsck": (fsck.stdout.strip().splitlines() or [fsck.stderr.strip()])[-1][:120],
    })
print(json.dumps(res))
