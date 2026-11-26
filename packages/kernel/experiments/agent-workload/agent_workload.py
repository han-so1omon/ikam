#!/usr/bin/env python3
"""E397: an AI agent's file work through `ikam mcp`: what each tool call
costs, how big the store grows against plain files and git for the same
history, and whether every version reads back byte for byte.

    python3 -I agent_workload.py IKAM_BIN WORK [SESSIONS] [SEED]

Builds a seeded project (Markdown notes, Python and Rust sources, CSV
tables, Word documents and a tar of logs; the office and tar files are
ingested first, since MCP tools carry text only), then runs SESSIONS agent
sessions through the official MCP Python SDK's client over one `ikam mcp`
process. Each session reads (ranges, search, passages, reads at an earlier
commit, checked against the model), edits code and notes, appends rows to
tables, writes a few generated files, and commits with its own message;
every fifth session works on a draft and promotes it. A model of every
commit's files is kept on the side. At the end every commit is checked out
and compared with the model byte for byte, and the same history is
replayed into git (`git gc`) for its size. Prints one JSON line. Local
files only.
"""
import asyncio
import io
import json
import os
import random
import shutil
import statistics
import subprocess
import sys
import tarfile
import time
import zipfile

from mcp import StdioServerParameters
from mcp.client.client import Client

WORDS = (
    "store version commit draft table column index query merge branch file "
    "record value change review search vector graph fragment schema report "
    "budget invoice shipment order customer region quarter metric target"
).split()


def sentence(r, n=12):
    return " ".join(r.choice(WORDS) for _ in range(n)).capitalize() + "."


def docx(text):
    """A minimal Word document whose body is `text`, one paragraph a line."""
    paras = "".join(f"<w:p><w:r><w:t>{line}</w:t></w:r></w:p>" for line in text.splitlines())
    doc = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">'
        f"<w:body>{paras}</w:body></w:document>"
    )
    b = io.BytesIO()
    with zipfile.ZipFile(b, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr(
            "[Content_Types].xml",
            '<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
            '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
            '<Default Extension="xml" ContentType="application/xml"/>'
            '<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>',
        )
        z.writestr(
            "_rels/.rels",
            '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
            '<Relationship Id="r1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>',
        )
        z.writestr("word/document.xml", doc)
    return b.getvalue()


def project(r):
    files = {}
    for i in range(40):
        body = "\n\n".join(" ".join(sentence(r) for _ in range(r.randint(3, 8))) for _ in range(r.randint(4, 12)))
        files[f"notes/n{i:03}.md"] = f"# Note {i}\n\n{body}\n".encode()
    for i in range(30):
        fns = "\n\n".join(
            f"def {r.choice(WORDS)}_{j}(x):\n    # {sentence(r, 6)}\n    return x * {j} + {r.randint(0, 99)}\n"
            for j in range(r.randint(5, 25))
        )
        files[f"src/py/m{i:03}.py"] = f'"""Module {i}."""\n\n{fns}'.encode()
    for i in range(20):
        fns = "\n".join(
            f"pub fn f{j}(x: u64) -> u64 {{\n    // {sentence(r, 6)}\n    x.wrapping_mul({r.randint(2, 999)})\n}}\n"
            for j in range(r.randint(5, 20))
        )
        files[f"src/rs/r{i:03}.rs"] = fns.encode()
    for i in range(15):
        rows = [f"{k},{r.choice(WORDS)},{r.randint(1, 9999)},{r.random():.4f}" for k in range(r.randint(200, 2000))]
        files[f"tables/t{i:02}.csv"] = ("id,name,qty,price\n" + "\n".join(rows) + "\n").encode()
    binary = {}
    for i in range(8):
        binary[f"office/d{i}.docx"] = docx("\n".join(sentence(r) for _ in range(r.randint(20, 80))))
    b = io.BytesIO()
    with tarfile.open(fileobj=b, mode="w") as t:
        for i in range(5):
            data = "\n".join(f"{k} {sentence(r, 8)}" for k in range(300)).encode()
            ti = tarfile.TarInfo(f"logs/day{i}.log")
            ti.size = len(data)
            ti.mtime = 0
            t.addfile(ti, io.BytesIO(data))
    binary["office/logs.tar"] = b.getvalue()
    return files, binary


class Timer:
    def __init__(self):
        self.ms = {}

    async def call(self, c, name, args):
        t = time.perf_counter()
        r = await c.call_tool(name, args)
        self.ms.setdefault(name, []).append((time.perf_counter() - t) * 1e3)
        text = "".join(x.text for x in r.content if getattr(x, "type", "") == "text")
        return text, r.is_error

    def summary(self):
        out = {}
        for k, v in sorted(self.ms.items()):
            v = sorted(v)
            out[k] = {
                "n": len(v),
                "p50": round(statistics.median(v), 2),
                "p99": round(v[min(len(v) - 1, int(len(v) * 0.99))], 2),
            }
        return out


def commit_id(text):
    # "committed ID on REF", "REF fast-forwarded to ID", "REF merged D as ID"
    return text.split()[1] if text.startswith("committed") else text.split()[-1]


async def sessions(ikam, store, n, r, files, binary):
    model = dict(files)
    model.update(binary)
    commits = []  # (id, files) in order
    head_files = None
    timer = Timer()
    read_checks = {"at_commit": 0, "equal": 0, "errors": 0}
    p = StdioServerParameters(command=ikam, args=["--store", store, "mcp"])
    async with Client(p, mode="legacy") as c:
        hist, _ = await timer.call(c, "history", {"limit": 1})
        first = hist.split()[0]
        commits.append((first, dict(model)))
        for s in range(n):
            on_draft = s % 5 == 4
            if on_draft:
                await timer.call(c, "draft_open", {"name": f"s{s}"})
            # Look around.
            await timer.call(c, "list", {"path": r.choice(["notes", "src/py", "tables", "office"])})
            await timer.call(c, "search", {"query": r.choice(WORDS) + " " + r.choice(WORDS)})
            await timer.call(c, "passages", {"question": sentence(r, 5)})
            path = r.choice([p for p in model if p.endswith((".md", ".py", ".rs"))])
            await timer.call(c, "read", {"path": path, "start_line": 1, "end_line": 40})
            await timer.call(c, "read", {"path": r.choice(list(binary))})  # extracted text
            # A read at an earlier commit, checked against the model.
            cid, old = r.choice(commits)
            cands = [p for p in old if p.endswith((".md", ".py", ".rs")) and len(old[p]) < 30000]
            path = r.choice(cands)
            text, err = await timer.call(c, "read", {"path": path, "at": cid})
            read_checks["at_commit"] += 1
            if err:
                read_checks["errors"] += 1
            else:
                body = text.split("\n", 1)[1] if "\n" in text else ""
                if body == old[path].decode():
                    read_checks["equal"] += 1
            # Edit code: rename one identifier that occurs once.
            for _ in range(r.randint(1, 4)):
                path = r.choice([p for p in model if p.endswith((".py", ".rs"))])
                text = model[path].decode()
                lines = [l for l in text.splitlines() if l.count("# ") == 1 or l.count("// ") == 1]
                line = r.choice(lines)
                if text.count(line) != 1:
                    continue
                new = line + " " + r.choice(WORDS)
                _, err = await timer.call(c, "edit", {"path": path, "old": line, "new": new})
                assert not err, path
                model[path] = text.replace(line, new, 1).encode()
            # Notes: rewrite a paragraph.
            path = r.choice([p for p in model if p.endswith(".md")])
            text = model[path].decode()
            paras = text.split("\n\n")
            i = r.randrange(1, len(paras))
            old_p = paras[i].rstrip("\n")
            if text.count(old_p) == 1 and old_p:
                new_p = " ".join(sentence(r) for _ in range(r.randint(2, 6)))
                _, err = await timer.call(c, "edit", {"path": path, "old": old_p, "new": new_p})
                assert not err
                model[path] = text.replace(old_p, new_p, 1).encode()
            # Tables: append rows (the agent writes the whole file).
            path = r.choice([p for p in model if p.endswith(".csv")])
            text = model[path].decode()
            k = text.count("\n")
            text += "".join(f"{k + j},{r.choice(WORDS)},{r.randint(1, 9999)},{r.random():.4f}\n" for j in range(r.randint(5, 50)))
            await timer.call(c, "write", {"path": path, "content": text})
            model[path] = text.encode()
            # A generated report now and then.
            if r.random() < 0.5:
                path = f"reports/s{s:03}.md"
                text = f"# Session {s}\n\n" + "\n".join(f"- {sentence(r, 8)}" for _ in range(r.randint(5, 30))) + "\n"
                await timer.call(c, "write", {"path": path, "content": text})
                model[path] = text.encode()
            await timer.call(c, "status", {})
            await timer.call(c, "diff", {})
            done, err = await timer.call(c, "commit", {"message": f"session {s}: edits, rows, report"})
            assert not err and done.startswith("committed"), done
            commits.append((commit_id(done), dict(model)))
            if on_draft:
                pr, err = await timer.call(c, "draft_promote", {"message": f"promote session {s}"})
                assert not err, pr
                commits.append((commit_id(pr), dict(model)))
            if s % 7 == 0:
                await timer.call(c, "history", {"limit": 5})
        head_files = dict(model)
    return commits, head_files, timer.summary(), read_checks


def du(path):
    return sum(os.path.getsize(os.path.join(d, f)) for d, _, fs in os.walk(path) for f in fs)


def checkout_all(ikam, store, work, commits):
    """Every commit checked out and compared with the model, byte for byte."""
    bad, checked = [], 0
    for cid, files in commits:
        dest = os.path.join(work, "co")
        shutil.rmtree(dest, ignore_errors=True)
        subprocess.run([ikam, "--store", store, "checkout", cid, dest], check=True, capture_output=True)
        got = {}
        for d, _, fs in os.walk(dest):
            for f in fs:
                p = os.path.join(d, f)
                got[os.path.relpath(p, dest)] = open(p, "rb").read()
        checked += 1
        if got != files:
            bad.append(cid[:12])
    shutil.rmtree(os.path.join(work, "co"), ignore_errors=True)
    return checked, bad


def git_size(work, commits):
    g = os.path.join(work, "git")
    shutil.rmtree(g, ignore_errors=True)
    os.makedirs(g)
    env = dict(os.environ, GIT_AUTHOR_NAME="e397", GIT_AUTHOR_EMAIL="e397@example.invalid",
               GIT_COMMITTER_NAME="e397", GIT_COMMITTER_EMAIL="e397@example.invalid")
    run = lambda *a: subprocess.run(["git", "-C", g, *a], check=True, capture_output=True, env=env)
    run("init", "-q")
    prev = {}
    seen = set()
    for i, (cid, files) in enumerate(commits):
        if cid in seen:
            continue
        seen.add(cid)
        for p in prev:
            if p not in files:
                os.remove(os.path.join(g, p))
        for p, b in files.items():
            if prev.get(p) != b:
                os.makedirs(os.path.dirname(os.path.join(g, p)), exist_ok=True)
                open(os.path.join(g, p), "wb").write(b)
        run("add", "-A")
        run("commit", "-q", "--allow-empty", "-m", f"c{i}")
        prev = files
    loose = du(os.path.join(g, ".git"))
    run("gc", "-q")
    return loose, du(os.path.join(g, ".git"))


def main():
    ikam, work = sys.argv[1:3]
    n = int(sys.argv[3]) if len(sys.argv) > 3 else 40
    seed = int(sys.argv[4]) if len(sys.argv) > 4 else 397
    r = random.Random(seed)
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)
    files, binary = project(r)
    src = os.path.join(work, "src")
    for p, b in {**files, **binary}.items():
        os.makedirs(os.path.dirname(os.path.join(src, p)), exist_ok=True)
        open(os.path.join(src, p), "wb").write(b)
    store = os.path.join(work, "store")
    subprocess.run([ikam, "--store", store, "ingest", src, "-m", "the project"], check=True, capture_output=True)
    after_ingest = du(store)
    t = time.perf_counter()
    commits, head, calls, reads = asyncio.run(sessions(ikam, store, n, r, files, binary))
    wall = time.perf_counter() - t
    after_sessions = du(store)
    subprocess.run([ikam, "--store", store, "repack"], check=True, capture_output=True)
    subprocess.run([ikam, "--store", store, "gc"], check=True, capture_output=True)
    after_repack = du(store)
    fsck = subprocess.run([ikam, "--store", store, "fsck"], capture_output=True, text=True)
    checked, bad = checkout_all(ikam, store, work, commits)
    git_loose, git_packed = git_size(work, commits)
    distinct = {}
    for _, fs in commits:
        for p, b in fs.items():
            distinct[(p, b)] = len(b)
    res = {
        "sessions": n,
        "seed": seed,
        "commits": len({c for c, _ in commits}),
        "calls": sum(v["n"] for v in calls.values()),
        "wall_s": round(wall, 1),
        "per_tool_ms": calls,
        "reads_at_commit": reads,
        "bytes": {
            "head_plain": sum(len(b) for b in head.values()),
            "every_version_plain": sum(distinct.values()),
            "store_after_ingest": after_ingest,
            "store_after_sessions": after_sessions,
            "store_after_repack_gc": after_repack,
            "git_loose": git_loose,
            "git_gc": git_packed,
        },
        "checkout": {"commits": checked, "mismatch": bad},
        "fsck_ok": fsck.returncode == 0,
    }
    print(json.dumps(res))


if __name__ == "__main__":
    main()
