#!/usr/bin/env python3
"""E359: does `ikam mcp` work with a standard MCP client, what does each tool
call cost, and how much text does an agent take in for one small edit?

    python3 mcp_check.py IKAM_BIN WORK_DIR CORPUS_DIR [COPIES] [distinct]

Needs the MCP Python SDK (`pip install mcp`). Stores CORPUS_DIR (COPIES times,
under c0/, c1/, ...; with distinct, each copy
after the first gets a line at the end of every file) in a new store under WORK_DIR, starts `ikam mcp` on it
through the SDK's stdio client, checks every tool, times calls, and prints
one JSON line. Local files only.
"""
import asyncio
import json
import os
import shutil
import statistics
import subprocess
import sys
import time

from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client


def build_store(ikam, work, corpus, copies, distinct):
    src = os.path.join(work, "corpus")
    store = os.path.join(work, "store")
    for d in (src, store):
        shutil.rmtree(d, ignore_errors=True)
    for i in range(copies):
        shutil.copytree(corpus, os.path.join(src, f"c{i}"))
        if distinct and i > 0:
            # A line at the end of every file, so no two copies share content.
            for root, _, names in os.walk(os.path.join(src, f"c{i}")):
                for n in names:
                    with open(os.path.join(root, n), "ab") as f:
                        f.write(f"\ncopy {i}\n".encode())
    t = time.perf_counter()
    subprocess.run([ikam, "--store", store, "ingest", src], check=True, capture_output=True)
    files, size = 0, 0
    for root, _, names in os.walk(src):
        for n in names:
            files += 1
            size += os.path.getsize(os.path.join(root, n))
    return store, src, files, size, time.perf_counter() - t


def text_of(r):
    return "".join(c.text for c in r.content if getattr(c, "type", "") == "text")


async def timed(session, name, args, n=1):
    times, out = [], None
    for _ in range(n):
        t = time.perf_counter()
        out = await session.call_tool(name, args)
        times.append((time.perf_counter() - t) * 1e3)
    return out, round(statistics.median(times), 2)


def pick_target(src):
    """A word that occurs exactly once in exactly one text file of one copy: the thing to change."""
    counts = {}
    for root, _, names in os.walk(src):
        for n in sorted(names):
            p = os.path.join(root, n)
            try:
                t = open(p, encoding="utf-8").read()
            except (UnicodeDecodeError, OSError):
                continue
            for w in set(t.split()):
                if len(w) >= 14 and w.isidentifier():
                    counts.setdefault(w, []).append((p, t.count(w)))
    for w in sorted(counts):
        hits = counts[w]
        if len(hits) == 1 and hits[0][1] == 1:
            return w, hits[0][0]
    raise SystemExit("no unique word found")


async def check(ikam, store, src, copies):
    params = StdioServerParameters(command=ikam, args=["--store", store, "mcp"])
    res = {}
    t0 = time.perf_counter()
    async with stdio_client(params) as (r, w):
        async with ClientSession(r, w) as session:
            init = await session.initialize()
            res["start_and_initialize_ms"] = round((time.perf_counter() - t0) * 1e3, 1)
            res["protocol"] = init.protocol_version
            tools = (await session.list_tools()).tools
            res["tools"] = len(tools)
            assert all(t.input_schema.get("type") == "object" for t in tools)

            word, path = pick_target(os.path.join(src, "c0"))
            rel = os.path.relpath(path, src)
            fsize = os.path.getsize(path)
            # The agent's path: search, read the lines around the hit, edit, commit.
            got = 0
            s, ms_search = await timed(session, "search", {"query": word}, 5)
            st = text_of(s)
            got += len(st)
            assert st.startswith(f"{copies} file(s)"), st[:200]
            assert st.split("\n")[1].startswith("c0/"), st[:200]
            line = int(st.split("\n")[2].split(":")[0])
            rd, ms_read = await timed(session, "read", {"path": rel, "start_line": max(1, line - 10), "end_line": line + 10}, 5)
            got += len(text_of(rd))
            new = word + "_e359"
            e, ms_edit = await timed(session, "edit", {"path": rel, "old": word, "new": new})
            assert not e.is_error, text_of(e)
            got += len(text_of(e))
            c, ms_commit = await timed(session, "commit", {"message": f"E359: rename {word}"})
            got += len(text_of(c))
            sent = len(word) * 2 + len(new) + len(rel) * 3 + 40
            res["task"] = {
                "file_bytes": fsize,
                "mcp_chars_in": got,
                "mcp_chars_out": sent,
                "whole_file_in": fsize,
                "whole_file_out": fsize,
                "files_only_scan_in": res.get("corpus_bytes", 0),
            }
            res["ms"] = {"search": ms_search, "read_21_lines": ms_read, "edit": ms_edit, "commit": ms_commit}

            h, res["ms"]["history_20"] = await timed(session, "history", {"limit": 20}, 5)
            assert f"E359: rename {word}" in text_of(h)
            ids = [x.split()[0] for x in text_of(h).split("\n") if x and not x.startswith(" ")]
            d, res["ms"]["diff_file"] = await timed(session, "diff", {"from": ids[1], "to": ids[0], "path": rel}, 3)
            dt = text_of(d)
            assert "+" in dt and new in dt, dt[:300]
            _, res["ms"]["list_root"] = await timed(session, "list", {}, 5)
            p, res["ms"]["passages"] = await timed(session, "passages", {"question": "how are drafts promoted into a branch"}, 5)
            res["passages_top"] = text_of(p).split("\n")[0]
            _, res["ms"]["status"] = await timed(session, "status", {}, 5)
            whole, res["ms"]["read_whole"] = await timed(session, "read", {"path": rel}, 5)
            wt = text_of(whole)
            assert new in wt or "read on with start_line=" in wt
            res["read_whole_chars"] = len(wt)

            # An edit of a small file, for the cost that does not come from its size.
            await session.call_tool("write", {"path": "small.txt", "content": "alpha beta\n" * 100})
            times = []
            for i in range(10):
                old, new2 = ("alpha", "gamma") if i % 2 == 0 else ("gamma", "alpha")
                t = time.perf_counter()
                await session.call_tool("edit", {"path": "small.txt", "old": old, "new": new2, "all": True})
                times.append((time.perf_counter() - t) * 1e3)
            res["ms"]["edit_small"] = round(statistics.median(times), 2)
            await session.call_tool("commit", {"message": "small edits"})

            # Drafts through the client.
            await session.call_tool("draft_open", {"name": "e359"})
            await session.call_tool("write", {"path": "draft-note.txt", "content": "only on the draft\n"})
            await session.call_tool("commit", {"message": "draft work"})
            on_main = await session.call_tool("read", {"path": "draft-note.txt", "at": "main"})
            assert on_main.is_error
            pr = await session.call_tool("draft_promote", {"message": "E359 draft"})
            assert not pr.is_error, text_of(pr)
            back = await session.call_tool("read", {"path": "draft-note.txt"})
            assert "only on the draft" in text_of(back)

            # Errors come back as tool errors, not protocol errors.
            bad = await session.call_tool("read", {"path": "no/such/file"})
            res["bad_path_is_error"] = bad.is_error
            # Leave one edit pending: closing stdin must commit it.
            await session.call_tool("write", {"path": "left-at-exit.txt", "content": "x\n"})
    out = subprocess.run([ikam, "--store", store, "log"], capture_output=True, text=True).stdout
    res["pending_committed_at_exit"] = "left at exit" in out
    return res


def main():
    ikam, work, corpus = sys.argv[1:4]
    copies = int(sys.argv[4]) if len(sys.argv) > 4 else 1
    distinct = len(sys.argv) > 5 and sys.argv[5] == "distinct"
    os.makedirs(work, exist_ok=True)
    store, src, files, size, ingest_s = build_store(ikam, work, corpus, copies, distinct)
    res = asyncio.run(check(ikam, store, src, copies))
    res["task"]["files_only_scan_in"] = size
    res.update({"copies": copies, "distinct": distinct, "files": files, "corpus_bytes": size, "ingest_s": round(ingest_s, 1)})
    print(json.dumps(res))


if __name__ == "__main__":
    main()
