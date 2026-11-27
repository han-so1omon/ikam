#!/usr/bin/env python3
"""E398: does `ikam live` and `ikam mcp` keep every edit they answered when
the process is killed with no warning (SIGKILL), and does the store come back
clean? (E374 asked this of `db serve` only.)

    python3 -I kill_live_mcp.py IKAM_BIN WORK TRIALS "MODE[:FLAGS];..."

MODE `live`: `ikam live PORT FLAGS` on a fresh store; a writer PUTs small
files over WebDAV without a pause (each answered 201 or 204 is
"acknowledged") while the process is killed after a random 0.2 to 3 s.
MODE `live-burst`: the same, but the writer pauses 0 to 3 s after bursts of
about 15 PUTs (a person saving, a sync tool), killed after 1 to 10 s. MODE `mcp`: `ikam mcp FLAGS`; a
scripted client writes a file and commits, over and over (a `committed ...`
answer is acknowledged), killed the same way. Then, on the same store:
  - `live`: every acknowledged PUT is in `main` right after the kill
    (`lost` counts those that are not), and after the process has started
    again and stopped cleanly (`lost_after_restart`), with its bytes; `mcp`: every acknowledged commit checks out exactly, and `main`
    is the last one acknowledged or the one in flight;
  - `ikam fsck` passes;
  - the process starts again and serves (restart time).
Prints one JSON line per mode. Local files and 127.0.0.1 only.
"""
import http.client
import json
import os
import random
import shutil
import signal
import subprocess
import sys
import threading
import time


def free_port():
    import socket
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def req(port, method, path, body=None, timeout=30):
    c = http.client.HTTPConnection("127.0.0.1", port, timeout=timeout)
    c.request(method, path, body=body)
    r = c.getresponse()
    data = r.read()
    c.close()
    return r.status, data


def wait_up(port, proc, limit=60):
    t = time.perf_counter()
    while time.perf_counter() - t < limit:
        if proc.poll() is not None:
            raise RuntimeError("exited %s" % proc.returncode)
        try:
            if req(port, "GET", "/dav/", timeout=2)[0] < 500:
                return time.perf_counter() - t
        except OSError:
            time.sleep(0.02)
    raise RuntimeError("not up")


def tree(ikam, store, rev, dest):
    shutil.rmtree(dest, ignore_errors=True)
    r = subprocess.run([ikam, "--store", store, "checkout", rev, dest], capture_output=True)
    if r.returncode:
        return None
    out = {}
    for d, _, fs in os.walk(dest):
        for f in fs:
            p = os.path.join(d, f)
            out[os.path.relpath(p, dest)] = open(p, "rb").read()
    shutil.rmtree(dest, ignore_errors=True)
    return out


def fsck(ikam, store):
    return subprocess.run([ikam, "--store", store, "fsck"], capture_output=True).returncode == 0


def seed(ikam, w):
    src = os.path.join(w, "seed")
    os.makedirs(src)
    open(os.path.join(src, "README.md"), "w").write("seed\n")
    store = os.path.join(w, "store")
    subprocess.run([ikam, "--store", store, "commit", src, "-m", "seed"], check=True, capture_output=True)
    return store


def trial_live(ikam, w, flags, r, burst=False):
    store = seed(ikam, w)
    port = free_port()
    p = subprocess.Popen([ikam, "--store", store, "live", str(port), *flags],
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    wait_up(port, p)
    assert req(port, "MKCOL", "/dav/w")[0] in (200, 201)
    acked, sent, stop, put_ms = {}, {}, threading.Event(), []

    def writer():
        n = 0
        while not stop.is_set():
            name = "w/f%05d.txt" % n
            body = ("%d %s\n" % (n, "x" * r.randint(10, 4000))).encode()
            sent[name] = body
            t0 = time.perf_counter()
            try:
                st, _ = req(port, "PUT", "/dav/" + name, body)
            except OSError:
                return
            put_ms.append((time.perf_counter() - t0) * 1e3)
            if st in (200, 201, 204):
                acked[name] = body
            n += 1
            if burst and r.random() < 1 / 15:
                stop.wait(r.uniform(0.0, 3.0))  # a pause after a burst

    th = threading.Thread(target=writer)
    th.start()
    time.sleep(r.uniform(1.0, 10.0) if burst else r.uniform(0.2, 3.0))
    p.send_signal(signal.SIGKILL)
    p.wait()
    stop.set()
    th.join()
    got = tree(ikam, store, "main", os.path.join(w, "co")) or {}
    lost = [k for k, b in acked.items() if got.get(k) != b]
    torn = [k for k in got if k.startswith("w/") and got[k] != sent.get(k)]
    ok = fsck(ikam, store)
    port = free_port()
    p = subprocess.Popen([ikam, "--store", store, "live", str(port), *flags],
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    up = wait_up(port, p)
    # What a client sees after the restart, before anything commits.
    seen = sum(1 for k, b in acked.items() if req(port, "GET", "/dav/" + k)[1] == b)
    p.send_signal(signal.SIGTERM)
    p.wait(timeout=60)
    # After the restart and a clean stop: what `main` keeps for good.
    kept = tree(ikam, store, "main", os.path.join(w, "co")) or {}
    lost_after = sum(1 for k, b in acked.items() if kept.get(k) != b)
    ok = ok and fsck(ikam, store)
    return {"acked": len(acked), "lost": len(lost), "lost_after_restart": lost_after,
            "torn": len(torn), "fsck": ok,
            "restart_s": round(up, 3), "served_after_restart": seen, "put_ms": put_ms}


class Mcp:
    def __init__(self, cmd):
        self.p = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL, text=True)
        self.n = 0

    def tool(self, name, args):
        self.n += 1
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.n, "method": "tools/call",
                                       "params": {"name": name, "arguments": args}}) + "\n")
        self.p.stdin.flush()
        line = self.p.stdout.readline()
        if not line:
            raise OSError("gone")
        return json.loads(line)["result"]["content"][0]["text"]


def trial_mcp(ikam, w, flags, r):
    store = seed(ikam, w)
    m = Mcp([ikam, "--store", store, "mcp", *flags])
    m.tool("status", {})
    model = {"README.md": b"seed\n"}
    acked, stop = [], threading.Event()
    inflight = {}

    def writer():
        n = 0
        try:
            while not stop.is_set():
                for j in range(r.randint(1, 5)):
                    name = "w/f%05d-%d.txt" % (n, j)
                    body = "%d %d %s\n" % (n, j, "y" * r.randint(10, 4000))
                    m.tool("write", {"path": name, "content": body})
                    model[name] = body.encode()
                inflight["files"] = dict(model)
                done = m.tool("commit", {"message": "c%d" % n})
                if done.startswith("committed"):
                    acked.append((done.split()[1], dict(model)))
                n += 1
        except (OSError, BrokenPipeError, ValueError):
            return

    th = threading.Thread(target=writer)
    th.start()
    time.sleep(r.uniform(0.2, 3.0))
    m.p.send_signal(signal.SIGKILL)
    m.p.wait()
    stop.set()
    th.join()
    bad = [c[:12] for c, fs in acked if tree(ikam, store, c, os.path.join(w, "co")) != fs]
    head = tree(ikam, store, "main", os.path.join(w, "co"))
    last = acked[-1][1] if acked else {"README.md": b"seed\n"}
    head_ok = head == last or head == inflight.get("files")
    ok = fsck(ikam, store)
    t = time.perf_counter()
    m = Mcp([ikam, "--store", store, "mcp", *flags])
    m.tool("status", {})
    up = time.perf_counter() - t
    m.p.stdin.close()
    m.p.wait(timeout=60)
    return {"acked": len(acked), "lost": len(bad), "head_ok": head_ok, "fsck": ok,
            "restart_s": round(up, 3)}


def main():
    ikam, work, trials, sets = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4].split(";")
    for spec in sets:
        mode, _, fl = spec.partition(":")
        flags = fl.split()
        r = random.Random(398)
        rows = []
        for k in range(trials):
            w = os.path.join(work, "%s-%d" % (mode, k))
            shutil.rmtree(w, ignore_errors=True)
            os.makedirs(w)
            if mode.startswith("live"):
                rows.append(trial_live(ikam, w, flags, r, burst=mode == "live-burst"))
            else:
                rows.append(trial_mcp(ikam, w, flags, r))
            shutil.rmtree(w, ignore_errors=True)
        agg = {"set": spec, "trials": trials,
               "acked": sum(x["acked"] for x in rows),
               "lost": sum(x["lost"] for x in rows),
               "trials_with_loss": sum(1 for x in rows if x["lost"]),
               "fsck_ok": sum(x["fsck"] for x in rows),
               "restart_s_max": max(x["restart_s"] for x in rows)}
        if mode.startswith("live"):
            agg["torn"] = sum(x["torn"] for x in rows)
            agg["lost_after_restart"] = sum(x["lost_after_restart"] for x in rows)
            agg["served_after_restart"] = sum(x["served_after_restart"] for x in rows)
            ms = sorted(v for x in rows for v in x.pop("put_ms"))
            agg["put_ms_p50_p99"] = [round(ms[len(ms) // 2], 2), round(ms[int(len(ms) * 0.99)], 2)] if ms else None
        else:
            agg["head_ok"] = sum(x["head_ok"] for x in rows)
        agg["rows"] = rows
        print(json.dumps(agg), flush=True)


if __name__ == "__main__":
    main()
