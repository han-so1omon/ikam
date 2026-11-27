#!/usr/bin/env python3
"""E434: binary record values over HTTP, written and read back byte-exact.

Usage: python3 -I records_b64.py IKAM_BIN [MAIN_BIN] WORK

Writes records with values that are not UTF-8 (random bytes, a float
array's bytes, invalid UTF-8) and some that are, through `ikam db serve`
(binary events), then reads them back over HTTP both ways:

1. `db serve`: frames (exact on main too), `get` (raw bytes),
   `format=json` (refused with 415 for a non-UTF-8 value) and
   `format=json&binary=base64` (E434);
2. `ikam host`: a records share of the same database read by another
   project through `/from/a/rec/records` (main: lossy; E434: base64 marker).

Every value is compared byte for byte with what was written. With MAIN_BIN,
the same requests go to main's binary too, and the answers for UTF-8 values
are compared byte for byte between the two binaries.
"""

import base64
import http.client
import json
import os
import random
import shutil
import socket
import struct
import subprocess
import sys
import time

EVENTS = "application/x-ikam-events"


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def wait(port, proc):
    for _ in range(400):
        if proc.poll() is not None:
            raise SystemExit("server exited: %s" % proc.returncode)
        try:
            socket.create_connection(("127.0.0.1", port), 0.2).close()
            return
        except OSError:
            time.sleep(0.05)
    raise SystemExit("server did not start")


def call(port, method, path, body=b"", headers=None):
    c = http.client.HTTPConnection("127.0.0.1", port, timeout=60)
    c.request(method, path, body, headers or {})
    r = c.getresponse()
    out = r.read()
    c.close()
    return r.status, out


def values():
    rnd = random.Random(434)
    floats = b"".join(struct.pack("<f", (i * 0.37 - 20.0)) for i in range(4096))
    doubles = b"".join(struct.pack("<d", rnd.gauss(0, 1)) for i in range(512))
    rows = {
        "b/random-16": bytes(rnd.getrandbits(8) for _ in range(16)),
        "b/random-64k": bytes(rnd.getrandbits(8) for _ in range(65536)),
        "b/f32-4096": floats,
        "b/f64-512": doubles,
        "b/content-id": bytes(rnd.getrandbits(8) for _ in range(32)),
        "b/invalid-ff": b"\xff",
        "b/invalid-lone-cont": b"\x80abc",
        "b/invalid-overlong": b"\xc0\xaf",
        "b/invalid-surrogate": b"\xed\xa0\x80",
        "b/invalid-truncated": "café".encode()[:4],
        "b/invalid-mixed": b"text \xfe\xfe then text",
        "b/nul-and-high": b"\x00\x01\x02\xf8\x88\x80\x80\x80",
        "u/empty": b"",
        "u/ascii": b"plain ascii text",
        "u/accents": "été über 中文 \U0001f600".encode(),
        "u/controls": b"tab\tnl\nquote\"back\\slash\x01",
        "u/marker-text": b'{"base64":"AAEC"}',
        "u/json": b'{"x":[1,2,3],"y":"z"}',
    }
    return dict(sorted(rows.items()))


def events(rows):
    out = b""
    for k, v in rows.items():
        out += b"\x00"
        for f in (k.encode(), ("w/" + k).encode(), v):
            out += struct.pack(">I", len(f)) + f
    return out


def unframe(b):
    out, i = {}, 0
    while i < len(b):
        n = struct.unpack(">I", b[i : i + 4])[0]
        k = b[i + 4 : i + 4 + n].decode()
        i += 4 + n
        n = struct.unpack(">I", b[i : i + 4])[0]
        out[k] = b[i + 4 : i + 4 + n]
        i += 4 + n
    return out


def value_bytes(v):
    """A client's read of one JSON value (E434's form)."""
    if isinstance(v, str):
        return v.encode()
    if isinstance(v, dict) and set(v) == {"base64"}:
        return base64.b64decode(v["base64"], validate=True)
    raise ValueError("unknown value form: %r" % (v,))


def pairs(body):
    return {k: value_bytes(v) for k, v in json.loads(body)}


def db_serve_reads(ikam, store, rows, write):
    port = free_port()
    p = subprocess.Popen([ikam, "--store", store, "db", "serve", str(port)],
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        wait(port, p)
        if write:
            st, b = call(port, "POST", "/db/rec/append", events(rows), {"content-type": EVENTS})
            assert st == 200, (st, b)
        res = {}
        st, b = call(port, "GET", "/db/rec/scan?prefix=")
        res["frames_exact"] = st == 200 and unframe(b) == rows
        res["get_exact"] = all(call(port, "GET", "/db/rec/get?key=" + k)[1] == v for k, v in rows.items())
        st, _ = call(port, "GET", "/db/rec/scan?prefix=b/&format=json")
        res["json_binary_status"] = st
        st, b = call(port, "GET", "/db/rec/scan?prefix=u/&format=json")
        res["json_utf8"] = (st, b)
        st, b = call(port, "GET", "/db/rec/scan?prefix=u/&format=json&binary=base64")
        res["json_utf8_b64"] = (st, b)
        st, b = call(port, "GET", "/db/rec/scan?prefix=&format=json&binary=base64")
        res["b64_status"] = st
        if st == 200:
            got = pairs(b)
            res["b64_exact"] = sum(got.get(k) == v for k, v in rows.items())
            res["b64_bytes"] = len(b)
            one = [call(port, "GET", "/db/rec/get?key=%s&format=json&binary=base64" % k) for k in rows]
            res["b64_get_exact"] = sum(
                s == 200 and pairs(body) == {k: v} for (s, body), (k, v) in zip(one, rows.items()))
            keys = "".join(k + "\n" for k in rows).encode()
            s, body = call(port, "POST", "/db/rec/get?format=json&binary=base64", keys)
            res["b64_post_get_exact"] = s == 200 and pairs(body) == rows
        return res
    finally:
        p.terminate()
        p.wait()


def passwd(ikam, user, pw):
    out = subprocess.run([ikam, "passwd", user], input=pw + "\n", capture_output=True,
                         text=True, check=True).stdout
    return out.strip().split("=", 1)[1]


def host_reads(ikam, work, store_a, rows, tag):
    store_b = os.path.join(work, "b-" + tag)
    os.makedirs(store_b, exist_ok=True)
    conf = os.path.join(work, "host-%s.conf" % tag)
    port = free_port()
    with open(conf, "w") as f:
        f.write("port %d\n" % port)
        for name, store in (("a", store_a), ("b", store_b)):
            f.write("project %s %s\n" % (name, store))
            f.write("host %s %s.example-host\n" % (name, name))
            f.write("user %s owner %s\n" % (name, passwd(ikam, name + "-owner", "pw")))
    p = subprocess.Popen([ikam, "host", "--config", conf, str(port)],
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    auth = lambda u: "Basic " + base64.b64encode((u + ":pw").encode()).decode()
    try:
        wait(port, p)
        h = {"Host": "a.localhost", "Authorization": auth("a-owner"), "X-Ikam": "1"}
        st, b = call(port, "POST", "/shares/issue?name=rec&to=b&scope=records:..&right=read&expires=600", b"", h)
        assert st == 200, (st, b)
        h = {"Host": "b.localhost", "Authorization": auth("b-owner")}
        res = {}
        st, b = call(port, "GET", "/from/a/rec/records", headers=h)
        res["status"] = st
        raw = json.loads(b)
        exact, lossy = 0, 0
        for k, v in raw:
            try:
                got = value_bytes(v)
            except ValueError:
                got = None
            if got == rows[k]:
                exact += 1
            elif isinstance(v, str) and "�" in v:
                lossy += 1
        res["exact"], res["lossy"], res["rows"] = exact, lossy, len(raw)
        res["bytes"] = len(b)
        st, b = call(port, "GET", "/from/a/rec/records?start=u/", headers=h)
        res["utf8"] = (st, b)
        return res
    finally:
        p.terminate()
        p.wait()


def main():
    args = sys.argv[1:]
    if len(args) not in (2, 3):
        raise SystemExit(__doc__)
    ikam, work = os.path.abspath(args[0]), args[-1]
    main_bin = os.path.abspath(args[1]) if len(args) == 3 else None
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)
    rows = values()
    store = os.path.join(work, "a")
    os.makedirs(store)
    out = {"rows": len(rows), "binary_rows": sum(k.startswith("b/") for k in rows)}
    # Write once with the branch's server, read with both.
    out["branch_db"] = db_serve_reads(ikam, store, rows, True)
    if main_bin:
        out["main_db"] = db_serve_reads(main_bin, store, rows, False)
    # The host shares project a's main ref: point it at the database's commit.
    shutil.copy(os.path.join(store, "refs", "db", "rec"), os.path.join(store, "refs", "main"))
    out["branch_host"] = host_reads(ikam, work, store, rows, "branch")
    if main_bin:
        out["main_host"] = host_reads(main_bin, work, store, rows, "main")
        out["db_utf8_identical"] = out["branch_db"]["json_utf8"] == out["main_db"]["json_utf8"]
        out["host_utf8_identical"] = out["branch_host"]["utf8"] == out["main_host"]["utf8"]
    out["db_utf8_same_with_option"] = out["branch_db"]["json_utf8"] == out["branch_db"]["json_utf8_b64"]
    for side in ("branch_db", "main_db", "branch_host", "main_host"):
        for k in ("json_utf8", "json_utf8_b64", "utf8"):
            if side in out and k in out[side]:
                st, b = out[side][k]
                out[side][k] = [st, len(b)]
    print(json.dumps(out, indent=1))


if __name__ == "__main__":
    main()
