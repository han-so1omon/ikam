#!/usr/bin/env python3
"""E114 self-test: check a running `ikam live --public` and publish the result.

    python3 -I realworld/selftest.py --url http://127.0.0.1:8739 \
        --auth-file AUTH [--commit SHA] [--history 200]

AUTH is a file holding `user:password` for the copy's IKAM_AUTH. Everything
the test writes goes under /selftest/ (cleared first), so it runs on a copy
that holds only test data. Checks, each timed:

- guests: the page and a file read without a password; every kind of write,
  the admin calls and a Claude answer refused or left out.
- files: plain text, CSV and a small Word file uploaded, read back byte for
  byte, listed by PROPFIND.
- search and ask: each file found by a word only it holds; an edit moves the
  word; ask's sources name the right file.
- edits: rename, copy, delete and a folder rename, then the listing checked.
- commit: the server's own commit, then the head moves.

Then it writes /results/latest.json (and appends a line to
/results/history.jsonl, keeping the last --history lines) with the password,
so anyone can read the outcome without it. The JSON never holds the password.
Prints the same JSON. Exit status 1 if any check failed.
"""

import argparse
import base64
import hashlib
import http.client
import io
import json
import socket
import sys
import time
import urllib.parse
import zipfile


class Client:
    def __init__(self, url, auth=None):
        u = urllib.parse.urlsplit(url)
        self.host, self.port = u.hostname, u.port or 80
        self.auth = auth
        self.c = None

    def conn(self):
        c = http.client.HTTPConnection(self.host, self.port, timeout=60)
        c.connect()
        return c

    def req(self, method, path, body=None, headers=None):
        h = {"Host": f"{self.host}:{self.port}", "X-Ikam": "1"}
        if self.auth:
            h["Authorization"] = "Basic " + base64.b64encode(self.auth.encode()).decode()
        h.update(headers or {})
        for attempt in range(2):
            try:
                if self.c is None:
                    self.c = self.conn()
                self.c.request(method, path, body=body, headers=h)
                r = self.c.getresponse()
                data = r.read()
                if r.getheader("connection", "").lower() == "close":
                    self.c = None
                return r.status, data
            except (http.client.HTTPException, OSError):
                self.c = None
                if attempt:
                    raise

    def dav(self, method, path, body=None, headers=None):
        return self.req(method, "/dav/" + urllib.parse.quote(path), body, headers)

    def api(self, name):
        s, b = self.req("GET", "/api/" + name)
        if s != 200:
            raise Check(f"/api/{name} answered {s}")
        return json.loads(b)


class Check(Exception):
    pass


def docx(text):
    """A minimal Word file holding `text`."""
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr(
            "[Content_Types].xml",
            '<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
            '<Default Extension="xml" ContentType="application/xml"/>'
            '<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>',
        )
        z.writestr(
            "word/document.xml",
            '<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">'
            f"<w:body><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:body></w:document>",
        )
    return buf.getvalue()


def files(run):
    """Test files, each holding one word no other file has."""
    out = {}
    for i in range(12):
        word = f"zq{run}t{i}w"  # no word a prefix of another
        out[f"selftest/text/note{i}.txt"] = (
            word,
            (f"Note {i} for the self-test. The marker is {word}.\n" + "filler line\n" * (i * 20)).encode(),
        )
    word = f"zq{run}csv"
    out["selftest/data/table.csv"] = (word, f"id,name\n1,{word}\n2,plain\n".encode())
    word = f"zq{run}doc"
    out["selftest/docs/letter.docx"] = (word, docx(f"A letter that mentions {word} once."))
    return out


def listing(cl, path):
    """Every path under `path`, by PROPFIND one level at a time."""
    s, b = cl.dav("PROPFIND", path + "/", headers={"Depth": "1"})
    if s != 207:
        raise Check(f"PROPFIND {path} answered {s}")
    out = set()
    for part in b.decode().split("<D:href>")[1:]:
        href = urllib.parse.unquote(part.split("</D:href>")[0]).removeprefix("/dav/")
        p = href.rstrip("/")
        if p == path:
            continue
        out.add(p)
        if href.endswith("/"):
            out |= listing(cl, p)
    return out


def search(cl, word):
    return sorted(h["path"] for h in cl.api("search?q=" + urllib.parse.quote(word))["hits"])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--url", default="http://127.0.0.1:8739")
    ap.add_argument("--auth-file", required=True)
    ap.add_argument("--commit", default="")
    ap.add_argument("--history", type=int, default=200)
    a = ap.parse_args()
    with open(a.auth_file) as f:
        auth = f.read().strip()
    guest, me = Client(a.url), Client(a.url, auth)
    run = format(int(time.time()) % 100000, "05d")
    result = {
        "commit": a.commit[:12],
        "started": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "checks": [],
        "timings_ms": {},
    }
    timings = {}

    def timed(name, ms):
        timings.setdefault(name, []).append(round(ms, 2))

    def check(name, fn):
        t = time.perf_counter()
        try:
            detail = fn()
            ok = True
        except Check as e:
            ok, detail = False, str(e)
        except (OSError, http.client.HTTPException, ValueError, KeyError) as e:
            ok, detail = False, f"{type(e).__name__}: {e}"
        entry = {"name": name, "ok": ok, "ms": round((time.perf_counter() - t) * 1e3, 1)}
        if detail:
            entry["detail"] = detail
        result["checks"].append(entry)
        return ok

    def expect(got, want, what):
        if got != want:
            raise Check(f"{what}: got {got}, expected {want}")

    def reachable():
        s, _ = guest.req("GET", "/api/status")
        expect(s, 200, "guest status")

    if not check("reachable", reachable):
        return finish(me, a, result, timings)

    def clear():
        s, _ = me.dav("DELETE", "selftest")
        if s not in (204, 404):
            raise Check(f"DELETE selftest answered {s}")
        for d in ("selftest", "selftest/text", "selftest/data", "selftest/docs", "results"):
            s, _ = me.dav("MKCOL", d)
            if s not in (201, 405):
                raise Check(f"MKCOL {d} answered {s}")

    check("clear the test area", clear)
    tests = files(run)

    def upload():
        for path, (_, body) in tests.items():
            t = time.perf_counter()
            s, _ = me.dav("PUT", path, body)
            timed("put", (time.perf_counter() - t) * 1e3)
            expect(s, 201, f"PUT {path}")
        return f"{len(tests)} files"

    check("upload", upload)

    def read_back():
        for path, (_, body) in tests.items():
            t = time.perf_counter()
            s, got = guest.dav("GET", path)
            timed("get", (time.perf_counter() - t) * 1e3)
            expect(s, 200, f"guest GET {path}")
            if hashlib.sha256(got).digest() != hashlib.sha256(body).digest():
                raise Check(f"{path} read back different bytes")

    check("read back byte for byte (as a guest)", read_back)

    def guests_cannot_write():
        tries = [
            ("PUT", "/dav/selftest/guest.txt", b"x"),
            ("DELETE", "/dav/selftest/data/table.csv", None),
            ("MKCOL", "/dav/selftest/guestdir", None),
            ("MOVE", "/dav/selftest/data/table.csv", None),
            ("LOCK", "/dav/selftest/data/table.csv", None),
            ("POST", "/api/commit", None),
            ("POST", "/api/repack", None),
            ("POST", "/api/fsck", None),
        ]
        for m, p, body in tries:
            h = {"Destination": "/dav/selftest/moved.csv"} if m == "MOVE" else None
            s, _ = guest.req(m, p, body, h)
            expect(s, 401, f"guest {m} {p}")
        s, b = guest.req("GET", "/api/ask?q=marker&llm=1")
        expect(s, 200, "guest ask")
        if "answer" in json.loads(b) or "error" in json.loads(b):
            raise Check("a guest got a Claude answer")
        s, _ = Client(a.url, auth.split(":")[0] + ":wrong").dav("GET", "selftest/data/table.csv")
        expect(s, 401, "a wrong password")
        return f"{len(tries)} writes refused"

    check("guests can read but not write", guests_cannot_write)

    def find_each():
        for path, (word, _) in tests.items():
            t = time.perf_counter()
            got = search(guest, word)
            timed("search", (time.perf_counter() - t) * 1e3)
            expect(got, [path], f"search {word}")

    check("search finds each file by its word", find_each)

    def ask():
        path = "selftest/text/note3.txt"
        word = tests[path][0]
        t = time.perf_counter()
        sources = guest.api("ask?q=" + urllib.parse.quote(f"which note has the marker {word}"))["sources"]
        timed("ask", (time.perf_counter() - t) * 1e3)
        if not sources or sources[0]["path"] != path:
            raise Check(f"ask's first source is {sources[0]['path'] if sources else None}, expected {path}")

    check("ask names the right source", ask)

    def edit():
        path = "selftest/text/note0.txt"
        old = tests[path][0]
        new = f"zq{run}edited"
        t = time.perf_counter()
        s, _ = me.dav("PUT", path, f"Rewritten. Now the marker is {new}.\n".encode())
        timed("edit", (time.perf_counter() - t) * 1e3)
        expect(s, 204, f"PUT over {path}")
        expect(search(guest, old), [], f"search {old} after the edit")
        expect(search(guest, new), [path], f"search {new} after the edit")

    check("an edit moves the word", edit)

    def moves():
        base = "/dav/selftest/"
        steps = [
            ("MOVE", "text/note1.txt", "text/renamed.txt", 201),
            ("COPY", "data/table.csv", "data/table-copy.csv", 201),
            ("DELETE", "text/note2.txt", None, 204),
            ("MOVE", "docs", "letters", 201),
        ]
        for m, src, dst, want in steps:
            h = {"Destination": base + dst} if dst else None
            t = time.perf_counter()
            s, _ = me.req(m, base + urllib.parse.quote(src), None, h)
            timed("change", (time.perf_counter() - t) * 1e3)
            expect(s, want, f"{m} {src}")
        got = listing(guest, "selftest")
        for p in ("selftest/text/renamed.txt", "selftest/data/table-copy.csv", "selftest/letters/letter.docx"):
            if p not in got:
                raise Check(f"{p} missing after the edits")
        for p in ("selftest/text/note1.txt", "selftest/text/note2.txt", "selftest/docs/letter.docx"):
            if p in got:
                raise Check(f"{p} still listed after the edits")
        expect(search(guest, tests["selftest/docs/letter.docx"][0]), ["selftest/letters/letter.docx"], "search after the folder rename")

    check("rename, copy, delete and folder rename", moves)

    def commit():
        before = me.api("status")["head"]
        t = time.perf_counter()
        s, b = me.req("POST", "/api/commit")
        timed("commit", (time.perf_counter() - t) * 1e3)
        expect(s, 200, "POST /api/commit")
        after = me.api("status")["head"]
        if after == before and not b.startswith(b"nothing"):
            raise Check("the head did not move")
        return b.decode()

    check("commit", commit)
    return finish(me, a, result, timings)


def finish(me, a, result, timings):
    for name, xs in timings.items():
        xs = sorted(xs)
        result["timings_ms"][name] = {"n": len(xs), "p50": xs[len(xs) // 2], "max": xs[-1]}
    try:
        st = me.api("status")
        result["store"] = {"files": st["files"], "store_bytes": st["store_bytes"], "head": (st["head"] or "")[:12]}
    except (Check, OSError, http.client.HTTPException, ValueError):
        pass
    result["ok"] = all(c["ok"] for c in result["checks"])
    result["failed"] = [c["name"] for c in result["checks"] if not c["ok"]]
    result["finished"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    line = json.dumps(result, separators=(",", ":"))
    try:
        me.dav("MKCOL", "results")
        me.dav("PUT", "results/latest.json", (json.dumps(result, indent=1) + "\n").encode())
        s, old = me.dav("GET", "results/history.jsonl")
        lines = old.decode().splitlines() if s == 200 else []
        lines = (lines + [line])[-a.history :]
        me.dav("PUT", "results/history.jsonl", ("\n".join(lines) + "\n").encode())
    except (OSError, http.client.HTTPException, socket.timeout) as e:
        print(f"could not publish the result: {e}", file=sys.stderr)
    print(line)
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
