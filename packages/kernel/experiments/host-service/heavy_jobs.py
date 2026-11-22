#!/usr/bin/env python3
"""E362: what one project's maintenance does to the others on `ikam host`,
with and without `heavy-jobs 1`; and a host's start time and memory by its
number of projects.

Fills projects a and b with FILES synthetic text files each and c with a
few, copies the stores, then for each config starts `ikam host` on a copy,
reads a file of c every 10 ms while a and b both start a repack (and then a
gc), and reports the reads' latency during the jobs, when each job ended
and the host's peak memory.

    python3 -I heavy_jobs.py IKAM_BIN WORK_DIR [FILES]

Prints JSON lines. Synthetic data only; 127.0.0.1 only.
"""
import base64
import json
import os
import random
import shutil
import socket
import statistics
import subprocess
import sys
import threading
import time
import urllib.request

PW = "e362-test"


def port_free():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def auth_line(binary, user):
    out = subprocess.run([binary, "passwd", user], input=PW + "\n", capture_output=True,
                         text=True, check=True).stdout.strip()
    return out.split("=", 1)[1]


class Host:
    def __init__(self, binary, work, projects, extra=""):
        self.binary, self.port = binary, port_free()
        lines = ["port %d" % self.port, extra]
        for name, store in projects:
            lines.append("project %s %s" % (name, store))
            lines.append("user %s owner %s" % (name, auth_line(binary, name + "-owner")))
        self.config = os.path.join(work, "host-%d.conf" % self.port)
        open(self.config, "w").write("\n".join(lines) + "\n")
        self.log = open(os.path.join(work, "host.log"), "ab")
        t0 = time.time()
        self.proc = subprocess.Popen([binary, "host", "--config", self.config],
                                     stdout=self.log, stderr=self.log)
        first = projects[0][0]
        while True:
            try:
                self.req(first, "GET", "/", None, 5)
                break
            except Exception:
                if time.time() - t0 > 300:
                    raise
                time.sleep(0.02)
        self.start_s = time.time() - t0

    def req(self, project, method, path, body, timeout=600):
        r = urllib.request.Request("http://127.0.0.1:%d%s" % (self.port, path), data=body,
                                   method=method)
        r.add_header("Host", "%s.localhost:%d" % (project, self.port))
        r.add_header("Authorization", "Basic " + base64.b64encode(
            ("%s-owner:%s" % (project, PW)).encode()).decode())
        if method == "POST":
            r.add_header("X-Ikam", "1")
        try:
            with urllib.request.urlopen(r, timeout=timeout) as resp:
                return resp.status, resp.read()
        except urllib.error.HTTPError as e:
            return e.code, e.read()

    def rss_mb(self):
        with open("/proc/%d/status" % self.proc.pid) as f:
            for line in f:
                if line.startswith("VmRSS"):
                    return int(line.split()[1]) / 1024

    def stop(self):
        self.proc.terminate()
        try:
            self.proc.wait(60)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        self.log.close()


WORDS = ["alpha", "beta", "gamma", "delta", "store", "graph", "chunk", "query", "merge",
         "index", "table", "event", "token", "share", "fold", "pack", "repack", "view"]


def text(rnd, i):
    base = " ".join(rnd.choice(WORDS) for _ in range(rnd.randrange(200, 600)))
    return ("# file %d\n\n%s\n" % (i, base)).encode()


def fill(binary, work, files):
    stores = {p: os.path.join(work, "fill", p) for p in "abc"}
    shutil.rmtree(os.path.join(work, "fill"), ignore_errors=True)
    for s in stores.values():
        os.makedirs(s)
    h = Host(binary, work, list(stores.items()))
    rnd = random.Random(362)
    t0 = time.time()
    for p, n in (("a", files), ("b", files), ("c", 20)):
        for i in range(n):
            st, _ = h.req(p, "PUT", "/dav/f%05d.md" % i, text(rnd, i))
            assert st in (201, 204), st
    time.sleep(4)  # the timed commit
    h.stop()
    return stores, time.time() - t0


def run(binary, work, stores, name, extra):
    d = os.path.join(work, name)
    shutil.rmtree(d, ignore_errors=True)
    copies = []
    for p, s in stores.items():
        c = os.path.join(d, p)
        shutil.copytree(s, c)
        copies.append((p, c))
    h = Host(binary, work, copies, extra)
    lat, stop = [], threading.Event()
    peak = [h.rss_mb()]

    def reader():
        while not stop.is_set():
            t = time.time()
            st, _ = h.req("c", "GET", "/dav/f00001.md", None, 60)
            assert st == 200, st
            lat.append((t, time.time() - t))
            time.sleep(0.01)

    def sampler():
        while not stop.is_set():
            peak[0] = max(peak[0], h.rss_mb())
            time.sleep(0.05)

    ends = {}

    def job(p, what):
        t = time.time()
        st, body = h.req(p, "POST", "/api/" + what, b"")
        ends[(p, what)] = (st, round(time.time() - t0, 2), round(time.time() - t, 2))

    threads = [threading.Thread(target=reader), threading.Thread(target=sampler)]
    for t in threads:
        t.start()
    time.sleep(1.0)
    quiet = [x for _, x in lat]
    t0 = time.time()
    for what in ("repack", "gc"):
        js = [threading.Thread(target=job, args=(p, what)) for p in "ab"]
        for j in js:
            j.start()
        for j in js:
            j.join()
    busy_end = time.time()
    stop.set()
    for t in threads:
        t.join()
    during = [x for t, x in lat if t0 <= t <= busy_end]
    ms = lambda xs: {"p50": round(statistics.median(xs) * 1000, 1),
                     "p99": round(sorted(xs)[int(len(xs) * 0.99) - 1] * 1000, 1),
                     "max": round(max(xs) * 1000, 1), "n": len(xs)}
    print(json.dumps({"config": name, "extra": extra, "jobs_s": round(busy_end - t0, 2),
                      "jobs": {"%s %s" % k: v for k, v in sorted(ends.items())},
                      "c_reads_quiet_ms": ms(quiet), "c_reads_during_ms": ms(during),
                      "peak_rss_mb": round(peak[0], 1)}), flush=True)
    h.stop()


def start_cost(binary, work, counts):
    for k in counts:
        d = os.path.join(work, "start-%d" % k)
        shutil.rmtree(d, ignore_errors=True)
        projects = []
        for i in range(k):
            s = os.path.join(d, "p%03d" % i)
            os.makedirs(s)
            projects.append(("p%03d" % i, s))
        h = Host(binary, work, projects)
        first = (h.start_s, h.rss_mb())
        h.stop()
        h = Host(binary, work, projects)
        print(json.dumps({"projects": k, "first_start_s": round(first[0], 2),
                          "start_s": round(h.start_s, 2), "rss_mb": round(h.rss_mb(), 1)}),
              flush=True)
        h.stop()


def main():
    binary, work = sys.argv[1], sys.argv[2]
    files = int(sys.argv[3]) if len(sys.argv) > 3 else 3000
    os.makedirs(work, exist_ok=True)
    stores, fill_s = fill(binary, work, files)
    print(json.dumps({"files_per_project": files, "fill_s": round(fill_s, 1)}), flush=True)
    for name, extra in (("no limit", ""), ("heavy-jobs 1", "heavy-jobs 1\nheavy-wait 600")):
        run(binary, work, stores, name.replace(" ", "-"), extra)
    start_cost(binary, work, [1, 10, 40])


if __name__ == "__main__":
    main()
