#!/usr/bin/env python3
"""E407: `ikam mcp --skills [PATH]`: are the listed digests the bytes served,
do store skills follow commits, are skills over the limits refused, and what
does the option cost at start?

    python3 mcp_skills_check.py IKAM_BIN WORK_DIR

A scripted JSON-RPC client (the official Python SDK has no skills client
yet) on a new store: lists the skills, gets each one, reads every file and
checks each `sha256:` digest and size with Python's own hashlib. Then edits
and commits a skill and lists again, adds skills over 512 files and over 16 MiB
and one whose folder and name differ, and measures the bytes of the start answers
with the option off and on, in both protocol eras. The official SDK's client
reads the built-in skill as a plain resource. Prints one JSON line. Local
files only.
"""
import asyncio
import base64
import hashlib
import json
import os
import shutil
import subprocess
import sys

from mcp import StdioServerParameters
from mcp.client.client import Client

META = {
    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
    "io.modelcontextprotocol/clientCapabilities": {},
}


class Server:
    def __init__(self, cmd):
        self.p = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.n = 0

    def raw(self, method, params=None):
        self.n += 1
        msg = {"jsonrpc": "2.0", "id": self.n, "method": method}
        if params is not None:
            msg["params"] = params
        self.p.stdin.write(json.dumps(msg) + "\n")
        self.p.stdin.flush()
        return self.p.stdout.readline()

    def rpc(self, method, params=None):
        a = json.loads(self.raw(method, params))
        if "error" in a:
            return None, a["error"]
        return a["result"], None

    def tool(self, name, args):
        r, e = self.rpc("tools/call", {"name": name, "arguments": args})
        assert e is None and not r["isError"], (name, r, e)
        return r["content"][0]["text"]

    def close(self):
        self.p.stdin.close()
        err = self.p.stderr.read()
        self.p.wait(timeout=120)
        return err


def check_all(s):
    """Every listed file read back: (files, bytes, mismatches, digests by uri)."""
    files, total, bad, digests = 0, 0, [], {}
    cursor = None
    while True:
        r, e = s.rpc("skills/list", {"cursor": cursor} if cursor else {})
        assert e is None, e
        for k in r["skills"]:
            got, e = s.rpc("skills/get", {"uri": k["uri"]})
            assert e is None and got["skill"] == k, (k["name"], e)
            for f in k["files"]:
                c, e = s.rpc("resources/read", {"uri": f["uri"]})
                assert e is None, e
                c = c["contents"][0]
                data = c["text"].encode() if "text" in c else base64.b64decode(c["blob"])
                want = "sha256:" + hashlib.sha256(data).hexdigest()
                if want != f["digest"] or len(data) != f["size"]:
                    bad.append(f["uri"])
                files += 1
                total += len(data)
                digests[f["uri"]] = f["digest"]
        cursor = r.get("nextCursor")
        if not cursor:
            return files, total, bad, digests, r.get("_meta", {}).get("ikam/refused", [])


def start_bytes(ikam, store, extra):
    """Bytes of the answers a client reads before its first call."""
    out = {}
    s = Server([ikam, "--store", store, "mcp", *extra])
    init = s.raw("initialize", {"protocolVersion": "2025-11-25", "capabilities": {}})
    tools = s.raw("tools/list")
    out["handshake"] = {"initialize": len(init), "tools_list": len(tools), "total": len(init) + len(tools)}
    disc = s.raw("server/discover", {"_meta": META})
    tools = s.raw("tools/list", {"_meta": META})
    out["2026-07-28"] = {"discover": len(disc), "tools_list": len(tools), "total": len(disc) + len(tools)}
    s.close()
    return out, init, json.loads(init)["result"]["instructions"]


async def sdk_reads_builtin(ikam, store):
    p = StdioServerParameters(command=ikam, args=["--store", store, "mcp", "--skills", "skills"])
    out = {}
    for mode in ("auto", "legacy"):
        async with Client(p, mode=mode) as c:
            res = await c.list_resources()
            r = await c.read_resource("skill://ikam-store/SKILL.md")
            out[mode] = {
                "version": c.session.protocol_version,
                "resources": len(res.resources),
                "skill_md_chars": len(r.contents[0].text),
            }
    return out


def main():
    ikam, work = sys.argv[1:3]
    shutil.rmtree(work, ignore_errors=True)
    src = os.path.join(work, "src")
    store = os.path.join(work, "store")
    # A small project with two skills of its own, one with a binary file.
    os.makedirs(os.path.join(src, "skills", "release-notes", "templates"))
    os.makedirs(os.path.join(src, "skills", "data-import"))
    with open(os.path.join(src, "README.md"), "w") as f:
        f.write("a project\n")
    with open(os.path.join(src, "skills", "release-notes", "SKILL.md"), "w") as f:
        f.write("---\nname: release-notes\ndescription: Write release notes from the history.\n---\n\nRead history, group by area.\n")
    with open(os.path.join(src, "skills", "release-notes", "templates", "notes.md"), "w") as f:
        f.write("## Changes\n\n- ...\n")
    with open(os.path.join(src, "skills", "data-import", "SKILL.md"), "w") as f:
        f.write("---\nname: data-import\ndescription: Import a CSV into the store.\n---\n\nUse write per file.\n")
    with open(os.path.join(src, "skills", "data-import", "sample.bin"), "wb") as f:
        f.write(bytes(range(256)) * 4)
    subprocess.run([ikam, "--store", store, "ingest", src], check=True, capture_output=True)

    res = {}
    s = Server([ikam, "--store", store, "mcp", "--skills", "skills"])
    files, total, bad, d1, refused = check_all(s)
    res["listed"] = {"files": files, "bytes": total, "digest_or_size_mismatch": bad, "refused": refused}

    # Edit a skill, see it unchanged until the commit, then with its new digest.
    uri = "skill://release-notes/SKILL.md"
    s.tool("edit", {"path": "skills/release-notes/SKILL.md", "old": "group by area", "new": "group by area and author"})
    pending = check_all(s)[3][uri]
    head_before = s.tool("history", {"limit": 1}).split()[0]
    s.tool("commit", {"message": "release-notes: by author too"})
    files2, _, bad2, d2, _ = check_all(s)
    old = s.tool("read", {"path": "skills/release-notes/SKILL.md", "at": head_before})
    res["edit"] = {
        "pending_not_listed": pending == d1[uri],
        "new_digest_after_commit": d2[uri] != d1[uri],
        "other_digests_unchanged": all(d2[u] == d1[u] for u in d1 if u != uri),
        "old_bytes_at_old_commit": "group by area." in old and "author" not in old,
        "mismatch": bad2,
    }

    # Three skills that must not be listed: over 512 files, over 16 MiB, and a
    # wrong name.
    s.tool("write", {"path": "skills/huge/SKILL.md", "content": "---\nname: huge\ndescription: too many files\n---\n"})
    for i in range(512):
        s.tool("write", {"path": f"skills/huge/parts/{i:03}.txt", "content": f"part {i}\n"})
    s.tool("write", {"path": "skills/misnamed/SKILL.md", "content": "---\nname: other-name\ndescription: x\n---\n"})
    s.tool("write", {"path": "skills/heavy/SKILL.md", "content": "---\nname: heavy\ndescription: too many bytes\n---\n"})
    s.tool("write", {"path": "skills/heavy/data.txt", "content": "x" * (16 << 20)})
    s.tool("commit", {"message": "three skills that are refused"})
    files3, _, bad3, d3, refused = check_all(s)
    _, err = s.rpc("skills/get", {"uri": "skill://huge/SKILL.md"})
    res["refused"] = {
        "listed_skills": sorted({u.split("/")[2] for u in d3}),
        "reasons": refused,
        "get_huge": err["message"] if err else None,
        "mismatch": bad3,
    }
    s.close()

    off, off_init, off_instr = start_bytes(ikam, store, [])
    on, on_init, on_instr = start_bytes(ikam, store, ["--skills", "skills"])
    res["start_bytes"] = {"off": off, "on": on}
    res["instructions_chars"] = {"off": len(off_instr), "on": len(on_instr)}
    res["sdk_client"] = asyncio.run(sdk_reads_builtin(ikam, store))
    print(json.dumps(res))


if __name__ == "__main__":
    main()
