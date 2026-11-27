#!/usr/bin/env python3
"""E418: several projects at once in one `ikam host`, one of them busy.

Seeds PROJECTS stores (p0 .. pN-1) of FILES synthetic text files each with
`ikam ingest`, plus a small project `q` for the limit checks, then for each
config starts `ikam host` on fresh copies and runs these phases while one
reader per project (its own process, one kept-alive connection) reads a
random file every 10 ms and, every tenth read, runs a small search:

  alone      each project's reader alone, 4 s each
  quiet      all readers together, 20 s
  import     p0 takes IMPORT new files from 4 writers (PUT, kept alive)
  flood      4 clients send p0 reads as fast as they can for 5 s with a
             read token (rated in the `limits` config)
  index      p0's first `rank` call (builds its BM25 file index)
  repack     p0 repacks
  repack2    p2 and p3 (one new file each; the last two with fewer than
             five projects) repack at the same time

During quiet and repack, p1 writes one file and the time until it is
committed is measured (the host's timed commit). The host's resident memory
is sampled every 50 ms. After the phases, project q checks its limits: a
body over max-body, and writes until max-store refuses them.

Configs: `none` (no limit lines), `limits` (heavy-jobs 1, a rate on p0's
token, max-body and max-store on q); `NAME+LINE|LINE` adds host config lines.

    python3 -I host_load.py IKAM_BIN WORK PROJECTS FILES IMPORT "CONFIG;CONFIG" [ROUNDS]

Prints JSON lines. Synthetic data only; 127.0.0.1 only.
"""
import base64
import http.client
import json
import multiprocessing as mp
import os
import random
import shutil
import socket
import subprocess
import sys
import threading
import time

PW = "e418-test"
WORDS = ("alpha beta gamma delta store graph chunk query merge index table event "
         "token share fold pack repack view river stone cloud field light paper "
         "north south east west orbit signal vector branch commit record").split()
FLOOD_RATE = 200  # requests a second for p0's token in `limits`
QUIET_S = 20  # the quiet phase, all readers and no heavy job


def port_free():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def text(rnd, i):
    body = " ".join(rnd.choice(WORDS) for _ in range(rnd.randrange(200, 600)))
    return ("# note %d\n\n%s\n" % (i, body)).encode()


def basic(user):
    return "Basic " + base64.b64encode(("%s:%s" % (user, PW)).encode()).decode()


class Conn:
    """One kept-alive connection to one project."""

    def __init__(self, port, project, auth):
        self.port, self.host, self.auth = port, "%s.localhost:%d" % (project, port), auth
        self.c = None

    def req(self, method, path, body=None, timeout=900):
        # A request is sent again only when a kept-alive connection had
        # closed before it was answered, and never after a timeout or for a
        # POST, so a slow repack is not asked for twice.
        for attempt in range(3):
            try:
                if self.c is None:
                    self.c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=timeout)
                elif self.c.sock is not None:
                    self.c.sock.settimeout(timeout)
                self.c.timeout = timeout
                h = {"Host": self.host, "Authorization": self.auth}
                if method == "POST":
                    h["X-Ikam"] = "1"
                self.c.request(method, path, body=body, headers=h)
                r = self.c.getresponse()
                data = r.read()
                if r.getheader("Connection", "").lower() == "close":
                    self.c.close()
                    self.c = None
                return r.status, data, r.getheader("Retry-After")
            except (http.client.HTTPException, OSError) as e:
                if self.c:
                    self.c.close()
                self.c = None
                if attempt == 2 or method == "POST" or isinstance(e, TimeoutError):
                    raise
                time.sleep(0.05)


def reader(port, project, files, active, stop, out):
    """Read a random file every 10 ms while `active`; every tenth, a search."""
    rnd = random.Random(sum(project.encode()))
    c = Conn(port, project, basic(project + "-owner"))
    rows, n = [], 0
    while not stop.is_set():
        if not active.is_set():
            time.sleep(0.005)
            continue
        n += 1
        if n % 10 == 0:
            kind, path = "search", "/api/search?q=%s+%s" % (rnd.choice(WORDS), rnd.choice(WORDS))
        else:
            kind, path = "file", "/dav/f%05d.md" % rnd.randrange(files)
        t = time.time()
        st, _, _ = c.req("GET", path)
        rows.append((t, kind, time.time() - t, st))
        time.sleep(0.01)
    out.send(rows)
    out.close()


def importer(port, project, count, writers, out):
    """PUT `count` new files from `writers` threads; statuses and time."""
    statuses, lock = {}, threading.Lock()
    nxt = [0]

    def run(w):
        c = Conn(port, project, basic(project + "-owner"))
        rnd = random.Random(4180 + w)
        while True:
            with lock:
                i = nxt[0]
                nxt[0] += 1
            if i >= count:
                return
            st, _, _ = c.req("PUT", "/dav/n%05d.md" % i, text(rnd, i))
            with lock:
                statuses[st] = statuses.get(st, 0) + 1

    t0 = time.time()
    ts = [threading.Thread(target=run, args=(w,)) for w in range(writers)]
    for t in ts:
        t.start()
    for t in ts:
        t.join()
    out.send({"s": round(time.time() - t0, 2), "statuses": statuses})
    out.close()


def flooder(port, project, token, secs, clients, out):
    statuses, lock = {}, threading.Lock()
    end = time.time() + secs

    def run():
        c = Conn(port, project, "Bearer " + token)
        while time.time() < end:
            st, _, _ = c.req("GET", "/dav/f00001.md")
            with lock:
                statuses[st] = statuses.get(st, 0) + 1

    ts = [threading.Thread(target=run) for _ in range(clients)]
    for t in ts:
        t.start()
    for t in ts:
        t.join()
    out.send(statuses)
    out.close()


def run_proc(target, *args):
    a, b = mp.Pipe(False)
    p = mp.Process(target=target, args=args + (b,))
    p.start()
    return p, a


def rss_mb(pid):
    try:
        with open("/proc/%d/status" % pid) as f:
            for line in f:
                if line.startswith("VmRSS:"):
                    return int(line.split()[1]) / 1024
    except OSError:
        return None


def pct(xs, p):
    xs = sorted(xs)
    return round(xs[min(len(xs) - 1, int(len(xs) * p))] * 1000, 2) if xs else None


def seed(binary, work, projects, files):
    root = os.path.join(work, "seed")
    shutil.rmtree(root, ignore_errors=True)
    stores = {}
    for k in range(projects):
        name = "p%d" % k
        src, store = os.path.join(root, "src-" + name), os.path.join(root, name)
        os.makedirs(src)
        rnd = random.Random(418 + k)
        for i in range(files):
            with open(os.path.join(src, "f%05d.md" % i), "wb") as f:
                f.write(text(rnd, i))
        t = time.time()
        subprocess.run([binary, "--store", store, "ingest", src, "-m", "seed"], check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        shutil.rmtree(src)
        stores[name] = store
        print(json.dumps({"seed": name, "files": files, "ingest_s": round(time.time() - t, 1)}),
              flush=True)
    q = os.path.join(root, "q")
    os.makedirs(q)
    stores["q"] = q
    return stores


def passwd(binary, user):
    out = subprocess.run([binary, "passwd", user], input=PW + "\n", capture_output=True,
                         text=True, check=True).stdout.strip()
    return out.split("=", 1)[1]


def make_token(binary):
    out = subprocess.run([binary, "token"], capture_output=True, text=True, check=True).stdout
    kv = dict(line.split("=", 1) for line in out.split())
    return kv["CONFIG"], kv["TOKEN"]


def run_config(binary, work, stores, spec, files, count, auths):
    name, _, extra = spec.partition("+")
    d = os.path.join(work, "run")
    shutil.rmtree(d, ignore_errors=True)
    projects = [p for p in stores if p != "q"]
    port = port_free()
    tok_conf, token = make_token(binary)
    lines = ["port %d" % port]
    limits = name == "limits"
    if limits:
        lines += ["heavy-jobs 1", "heavy-wait 900"]
    for p, s in stores.items():
        c = os.path.join(d, p)
        shutil.copytree(s, c)
        lines += ["project %s %s" % (p, c), "user %s owner %s" % (p, auths[p])]
    lines.append("token p0 read %s%s" % (tok_conf, " rate=%d/s" % FLOOD_RATE if limits else ""))
    if limits:
        lines += ["max-body q 65536", "max-store q 2M"]
    lines += [x for x in extra.split("|") if x]
    conf = os.path.join(work, "host.conf")
    open(conf, "w").write("\n".join(lines) + "\n")
    log = open(os.path.join(work, "host.log"), "ab")
    t0 = time.time()
    proc = subprocess.Popen([binary, "host", "--config", conf], stdout=log, stderr=log)
    ctl = {p: Conn(port, p, basic(p + "-owner")) for p in stores}
    while True:
        try:
            if ctl["p0"].req("GET", "/api/status", timeout=5)[0] == 200:
                break
        except OSError:
            pass
        if time.time() - t0 > 600:
            raise SystemExit("host did not start")
        time.sleep(0.05)
    start_s = time.time() - t0
    start_rss = rss_mb(proc.pid)

    rss, stop_s = [], threading.Event()

    def sampler():
        while not stop_s.is_set():
            r = rss_mb(proc.pid)
            if r:
                rss.append((time.time(), r))
            time.sleep(0.05)

    threading.Thread(target=sampler, daemon=True).start()
    stop = mp.Event()
    active = {p: mp.Event() for p in projects}
    readers = {p: run_proc(reader, port, p, files, active[p], stop) for p in projects}
    phases = []

    def phase(label, fn):
        t = time.time()
        info = fn() or {}
        phases.append((label, t, time.time(), info))

    def hold(secs):
        return lambda: time.sleep(secs)

    def head(p):
        # The ref file on disk, read without asking the host (asking takes
        # the project's lock, and /api/status measures the whole store).
        try:
            return open(os.path.join(d, p, "refs", "main"), "rb").read()
        except OSError:
            return None

    def lag_write(p):
        """Write one file to p and time until the host's timed commit takes it."""
        before, t = head(p), time.time()
        st, _, _ = ctl[p].req("PUT", "/dav/lag%d.md" % int(t * 1000), b"lag check\n")
        while head(p) == before:
            time.sleep(0.05)
        return {"commit_lag_s": round(time.time() - t, 2), "put": st}

    time.sleep(0.5)
    for p in projects:
        active[p].set()
        phase("alone " + p, hold(4))
        active[p].clear()
    for p in projects:
        active[p].set()

    def quiet():
        time.sleep(2)
        info = lag_write("p1")
        time.sleep(max(0, QUIET_S - 2 - info["commit_lag_s"]))
        return info
    phase("quiet", quiet)

    def imp():
        pr, conn = run_proc(importer, port, "p0", count, 4)
        r = conn.recv()
        pr.join()
        return {"import_s": r["s"], "statuses": r["statuses"],
                "files_per_s": round(count / r["s"], 1)}
    phase("import", imp)
    time.sleep(3)  # p0's timed commit after the import

    def flood():
        pr, conn = run_proc(flooder, port, "p0", token, 5.0, 4)
        r = conn.recv()
        pr.join()
        ok = r.get(200, 0)
        return {"statuses": r, "answered_per_s": round(ok / 5.0, 1)}
    phase("flood", flood)

    def index():
        t = time.time()
        st, body, _ = ctl["p0"].req("GET", "/api/tool/rank?query=river+stone")
        return {"status": st, "rank_s": round(time.time() - t, 2)}
    phase("index", index)

    def repack():
        res = {}
        th = threading.Thread(target=lambda: res.update(
            r=ctl["p0"].req("POST", "/api/repack", b"")))
        t = time.time()
        th.start()
        time.sleep(1)
        lag = lag_write("p1")
        th.join()
        lag.update(status=res["r"][0], repack_s=round(time.time() - t, 2),
                   msg=res["r"][1].decode()[:120])
        return lag
    phase("repack", repack)

    def repack2():
        # Two projects each get one new file (so a repack has work), then
        # both repack at once; the others keep reading.
        pair = projects[2:4] if len(projects) >= 5 else projects[-2:]
        for p in pair:
            lag_write(p)
        ends = {}

        def job(p):
            t = time.time()
            st, body, _ = ctl[p].req("POST", "/api/repack", b"")
            ends[p] = (st, round(t - t2, 2), round(time.time() - t2, 2))
        t2 = time.time()
        ths = [threading.Thread(target=job, args=(p,)) for p in pair]
        for th in ths:
            th.start()
        for th in ths:
            th.join()
        return {"busy": pair,
                "jobs": {p: {"status": v[0], "start_s": v[1], "end_s": v[2]}
                         for p, v in sorted(ends.items())}}
    phase("repack2", repack2)

    stop.set()
    samples = {}
    for p, (pr, conn) in readers.items():
        samples[p] = conn.recv()
        pr.join()
    status = {p: json.loads(ctl[p].req("GET", "/api/status")[1]) for p in projects}

    # Project q's own limits.
    limit, tq = {}, time.time()
    st, _, _ = ctl["q"].req("PUT", "/dav/big.md", b"x" * 100_000)
    limit["put_100k"] = st
    rnd, codes, n = random.Random(7), {}, 0
    for i in range(400):
        st, _, _ = ctl["q"].req("PUT", "/dav/q%04d.md" % i,
                                bytes(rnd.randrange(256) for _ in range(20_000)))
        codes[st] = codes.get(st, 0) + 1
        if st in (201, 204):
            n += 1
        elif st == 507 and codes[507] >= 3:
            break
    time.sleep(2)
    qs = json.loads(ctl["q"].req("GET", "/api/status")[1])
    limit.update(puts=codes, accepted=n, store_bytes=qs["store_bytes"])
    others = {p: ctl[p].req("PUT", "/dav/after-q.md", b"still writes\n")[0] for p in projects}
    limit["other_projects_put_after_q"] = others
    peak_q = [r for t, r in rss if t >= tq]
    limit["rss_before_mb"] = round(max(r for t, r in rss if t < tq), 1)
    limit["rss_peak_mb"] = round(max(peak_q), 1) if peak_q else None

    stop_s.set()
    proc.terminate()
    try:
        proc.wait(120)
    except subprocess.TimeoutExpired:
        proc.kill()
    log.close()

    out = {"config": spec, "projects": len(projects), "files": files, "import": count,
           "start_s": round(start_s, 2), "start_rss_mb": round(start_rss or 0, 1),
           "text_mb_end": round(sum(s["text_bytes"] for s in status.values()) / 1e6, 1),
           "store_mb_end": round(sum(s["store_bytes"] for s in status.values()) / 1e6, 1),
           "limits_q": limit, "phases": []}
    for label, a, b, info in phases:
        row = {"phase": label, "s": round(b - a, 2), **info}
        peak = [r for t, r in rss if a <= t <= b]
        row["rss_peak_mb"] = round(max(peak), 1) if peak else None
        for p, xs in samples.items():
            if label.startswith("alone") and label != "alone " + p:
                continue
            f = [x for t, k, x, s in xs if a <= t <= b and k == "file"]
            q = [x for t, k, x, s in xs if a <= t <= b and k == "search"]
            bad = sum(1 for t, k, x, s in xs if a <= t <= b and s != 200)
            row[p] = {"n": len(f), "p50": pct(f, 0.5), "p99": pct(f, 0.99),
                      "max": round(max(f) * 1000, 1) if f else None,
                      "search_p50": pct(q, 0.5), "search_p99": pct(q, 0.99), "not_200": bad}
        if not label.startswith("alone"):
            # The projects the phase's job is not in, pooled (p0 is the busy
            # one but in repack2; the quiet phase pools the same p1..).
            busy = info.get("busy", ["p0"])
            f = [x for p, xs in samples.items() if p not in busy
                 for t, k, x, s in xs if a <= t <= b and k == "file"]
            q = [x for p, xs in samples.items() if p not in busy
                 for t, k, x, s in xs if a <= t <= b and k == "search"]
            row["others"] = {"n": len(f), "p50": pct(f, 0.5), "p99": pct(f, 0.99),
                             "p999": pct(f, 0.999), "search_p50": pct(q, 0.5),
                             "search_p99": pct(q, 0.99)}
        out["phases"].append(row)
    base = next(r["others"] for r in out["phases"] if r["phase"] == "quiet")
    for r in out["phases"]:
        if "others" in r and base["p99"]:
            r["others"]["p99_vs_quiet"] = round(r["others"]["p99"] / base["p99"], 2)
    out["rss_peak_mb"] = round(max(r for _, r in rss), 1)
    out["rss_end_mb"] = round(rss[-1][1], 1)
    print(json.dumps(out), flush=True)


def main():
    binary, work = sys.argv[1], sys.argv[2]
    projects, files, count = int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5])
    specs = sys.argv[6].split(";")
    rounds = int(sys.argv[7]) if len(sys.argv) > 7 else 1
    os.makedirs(work, exist_ok=True)
    stores = seed(binary, work, projects, files)
    auths = {p: passwd(binary, p + "-owner") for p in stores}
    for r in range(rounds):
        for spec in specs:
            run_config(binary, work, stores, spec, files, count, auths)
    shutil.rmtree(os.path.join(work, "run"), ignore_errors=True)


if __name__ == "__main__":
    mp.set_start_method("fork")
    main()
