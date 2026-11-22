#!/usr/bin/env python3
"""E370: two `ikam db serve` processes on one store directory.

    python3 -I two_servers.py IKAM_BIN WORK_DIR [BATCHES] [FLAGS...]

With SOLO=1 in the environment only A appends (B runs and stays idle), to
see what the option costs a store with one writer.

Server A loads 2,000 rows `t/N` (short text) and searches them. Then A and
B, each from its own thread, append BATCHES batches of 50 new rows (keys
of their own), and after each batch read one of the other server's rows
and run a search. A third server, started after, counts the rows. Prints
one JSON line: requests that failed (by kind), rows expected and found,
time, and each append's median and slowest latency. FsStore only.
"""
import http.client
import json
import os
import shutil
import socket
import statistics
import struct
import subprocess
import sys
import threading
import time

BIN, WORK = sys.argv[1], sys.argv[2]
BATCHES = int(sys.argv[3]) if len(sys.argv) > 3 else 40
FLAGS = sys.argv[4:]
EV = "application/x-ikam-events"


def pack(*parts):
    return b"".join(struct.pack(">I", len(p)) + p for p in parts)


def upsert(key, val, idem):
    return b"\x00" + pack(key.encode(), idem.encode(), val)


def port_free():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


class Server:
    def __init__(self, store):
        self.port = port_free()
        log = open(os.path.join(WORK, "db.log"), "ab")
        self.proc = subprocess.Popen([BIN, "--store", store, "db", "serve", *FLAGS,
                                      str(self.port)], stdout=log, stderr=log)
        for _ in range(400):
            try:
                self.conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=300)
                self.req("GET", "/db/")
                return
            except OSError:
                time.sleep(0.05)
        raise SystemExit("server did not start")

    def req(self, method, path, body=None, ctype=None):
        h = {"Host": f"127.0.0.1:{self.port}"}
        if ctype:
            h["Content-Type"] = ctype
        self.conn.request(method, path, body=body, headers=h)
        r = self.conn.getresponse()
        return r.status, r.read()

    def stop(self):
        self.proc.terminate()
        self.proc.wait()


def text(i, w):
    return f"row {i} about {w} and json".encode()


store = os.path.join(WORK, "store")
shutil.rmtree(store, ignore_errors=True)
a = Server(store)
st, _ = a.req("POST", "/db/s/append",
              b"".join(upsert(f"t/{i:06}", text(i, "maps"), f"l{i}") for i in range(2000)), EV)
assert st == 200, st
assert a.req("GET", "/db/s/search?q=json+maps&prefix=t/&k=5")[0] == 200
b = Server(store)
fails = {"append": 0, "get": 0, "search": 0}
lat = []
lock = threading.Lock()


def work(srv, base, other):
    for n in range(BATCHES):
        keys = range(base + n * 50, base + n * 50 + 50)
        body = b"".join(upsert(f"t/{i:06}", text(i, "sound"), f"w{i}") for i in keys)
        t = time.perf_counter()
        s1, _ = srv.req("POST", "/db/s/append", body, EV)
        dt = time.perf_counter() - t
        s2, _ = srv.req("GET", f"/db/s/get?key=t/{other + max(n - 1, 0) * 50:06}")
        s3, _ = srv.req("GET", "/db/s/search?q=json+sound&prefix=t/&k=5")
        with lock:
            lat.append(dt)
            fails["append"] += s1 != 200
            # The other's row may not be written yet on the first batch.
            fails["get"] += s2 not in (200, 404)
            fails["search"] += s3 != 200


t = time.perf_counter()
ts = [threading.Thread(target=work, args=(a, 100000, 100000))]
if not os.environ.get("SOLO"):
    ts.append(threading.Thread(target=work, args=(b, 200000, 100000)))
for x in ts:
    x.start()
for x in ts:
    x.join()
secs = time.perf_counter() - t
a.stop()
b.stop()
c = Server(store)
st, body = c.req("GET", "/db/s/count?prefix=t/")
c.stop()
found = json.loads(body)["count"] if st == 200 else None
print(json.dumps({"flags": FLAGS, "batches": BATCHES, "fails": fails,
                  "rows_expected": 2000 + len(ts) * BATCHES * 50, "rows_found": found,
                  "s": round(secs, 1), "append_p50_ms": round(statistics.median(lat) * 1000, 1),
                  "append_max_ms": round(max(lat) * 1000, 1)}), flush=True)
