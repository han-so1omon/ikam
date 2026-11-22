#!/usr/bin/env python3
"""E354: does `ikam db serve` work as a live database with its defaults?

Loads a table of N rows (with one index entry per row) in batches, stops the
server, starts it again, and times the first append of a batch into a new
table (the case a restarted server pays the most for), then a few more.
Runs once per flag set. A client gives up after TIMEOUT seconds, as a
typical HTTP client would.

    python3 db_first_write.py IKAM_BIN WORK_DIR [ROWS] [BATCH]

Prints one JSON line per flag set. Synthetic data only; 127.0.0.1 only.
"""
import json
import os
import shutil
import socket
import struct
import subprocess
import sys
import time
import urllib.error
import urllib.request
import base64
import random

TIMEOUT = 120.0
BASE = ["--packs", "--plain-events", "--no-object-templates"]
SETS = {
    "defaults": [],
    "base": BASE,
    "base+write-options": BASE + ["--edit-index", "--concurrent-reads"],
    "live": ["--live"],
    "live+plain-writes": ["--live", "--plain-writes"],
}
PASSWORD = "e354-test"


def port_free():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def pack(*parts):
    return b"".join(struct.pack(">I", len(p)) + p for p in parts)


def events(table, start, n, run):
    out = []
    rnd = random.Random(start)
    for i in range(start, start + n):
        pk = b"p%019d" % i
        val = json.dumps([i, "2026-01-%02d" % (1 + i % 28), "word%d" % rnd.randrange(5000),
                          rnd.random() * 1000, "x" * rnd.randrange(40, 160)]).encode()
        key = b"t/" + table + b"/" + pk
        out.append(b"\x00" + pack(key, b"%s-%d" % (run, i), val))
        ikey = b"i/" + table + b"/by_word/" + val.split(b'"')[3] + b"/" + pk
        out.append(b"\x00" + pack(ikey, b"%s-i%d" % (run, i), pk))
    return b"".join(out)


class Server:
    def __init__(self, binary, store, flags):
        self.binary, self.store, self.flags = binary, store, flags
        self.port = port_free()
        self.auth = None
        self.proc = None

    def start(self):
        if self.auth is None:
            line = subprocess.run([self.binary, "passwd", "u"], input=PASSWORD + "\n",
                                  capture_output=True, text=True, check=True).stdout.strip()
            self.auth = line.split("=", 1)[1] if line.startswith("IKAM_AUTH=") else line
        env = dict(os.environ, IKAM_AUTH=self.auth)
        self.log = open(os.path.join(os.path.dirname(self.store), "db.log"), "ab")
        self.proc = subprocess.Popen([self.binary, "--store", self.store, "db", "serve",
                                      *self.flags, str(self.port)], env=env,
                                     stdout=self.log, stderr=self.log)
        t0 = time.time()
        while time.time() - t0 < 60:
            try:
                self.req("GET", "/db/t/head", None, 5)
                return time.time() - t0
            except Exception:
                time.sleep(0.1)
        raise RuntimeError("server did not start")

    def stop(self):
        self.proc.terminate()
        try:
            self.proc.wait(60)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait()
        self.log.close()

    def req(self, method, path, body, timeout):
        r = urllib.request.Request("http://127.0.0.1:%d%s" % (self.port, path), data=body,
                                   method=method)
        r.add_header("Authorization", "Basic " + base64.b64encode(
            ("u:" + PASSWORD).encode()).decode())
        if body is not None:
            r.add_header("Content-Type", "application/x-ikam-events")
        with urllib.request.urlopen(r, timeout=timeout) as resp:
            return resp.read()

    def append(self, body, timeout=TIMEOUT):
        t0 = time.time()
        try:
            self.req("POST", "/db/t/append", body, timeout)
            return time.time() - t0, True
        except (socket.timeout, TimeoutError, urllib.error.URLError) as e:
            return time.time() - t0, False


def du(path):
    total = 0
    for root, _, files in os.walk(path):
        for f in files:
            try:
                total += os.path.getsize(os.path.join(root, f))
            except OSError:
                pass
    return total


def run(binary, work, name, flags, rows, batch):
    d = os.path.join(work, name)
    shutil.rmtree(d, ignore_errors=True)
    os.makedirs(d)
    store = os.path.join(d, "store")
    s = Server(binary, store, flags)
    s.start()
    load = []
    t0 = time.time()
    for start in range(0, rows, batch):
        dt, ok = s.append(events(b"big", start, min(batch, rows - start), b"load"), timeout=600)
        load.append(dt)
        if not ok:
            break
    load_s = time.time() - t0
    s.stop()
    restart_s = s.start()
    first, ok = s.append(events(b"new", 0, batch, b"after"))
    later = []
    if ok:
        for k in range(1, 6):
            dt, ok2 = s.append(events(b"new", k * batch, batch, b"after"))
            later.append(dt)
    big_add, ok3 = s.append(events(b"big", rows, batch, b"more")) if ok else (None, None)
    s.stop()
    later.sort()
    return {
        "flags": name, "rows": rows, "batch": batch,
        "load_s": round(load_s, 2), "load_batch_p50_ms": round(sorted(load)[len(load) // 2] * 1000, 1),
        "restart_s": round(restart_s, 2),
        "first_write_s": round(first, 3), "first_write_ok": ok,
        "next_writes_p50_ms": round(later[len(later) // 2] * 1000, 1) if later else None,
        "append_to_big_s": round(big_add, 3) if big_add is not None else None,
        "store_mb": round(du(store) / 1e6, 1),
        "timeout_s": TIMEOUT,
    }


def main():
    binary, work = sys.argv[1], sys.argv[2]
    rows = int(sys.argv[3]) if len(sys.argv) > 3 else 100_000
    batch = int(sys.argv[4]) if len(sys.argv) > 4 else 2000
    only = sys.argv[5].split(",") if len(sys.argv) > 5 else list(SETS)
    for name in only:
        print(json.dumps(run(binary, work, name, SETS[name], rows, batch)), flush=True)


if __name__ == "__main__":
    main()
