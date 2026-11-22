"""E360: the agent tools over HTTP on `ikam live`. Loads the .md and .rs files
of CORPUS through /api/tool/write, then times reads, search, edits, commits,
history, a diff and draft open/commit/promote cycles. Usage:
http_check.py IKAM_BIN WORK CORPUS"""
import json, os, subprocess, sys, time, urllib.request, statistics, shutil, socket
binp, work, src = sys.argv[1], sys.argv[2], sys.argv[3]
shutil.rmtree(work, ignore_errors=True); os.makedirs(work)
store = os.path.join(work, "store")
s = socket.socket(); s.bind(("127.0.0.1", 0)); port = s.getsockname()[1]; s.close()
p = subprocess.Popen([binp, "--store", store, "live", str(port)], stdout=subprocess.DEVNULL, stderr=open(os.path.join(work, "live.log"), "w"))
import atexit; atexit.register(lambda: p.poll() is None and p.kill())
base = f"http://127.0.0.1:{port}"
def call(method, path, body=None):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(base + path, data=data, method=method, headers={"X-Ikam": "1"})
    t = time.perf_counter()
    try:
        with urllib.request.urlopen(req) as r: st, out = r.status, r.read()
    except urllib.error.HTTPError as e: st, out = e.code, e.read()
    return st, out, (time.perf_counter() - t) * 1000
for _ in range(100):
    try: call("GET", "/api/tools"); break
    except Exception: time.sleep(0.1)
files = []
for root, _, names in os.walk(src):
    for n in names:
        if n.endswith((".md", ".rs")):
            files.append(os.path.join(root, n))
t = time.perf_counter()
for f in files:
    rel = os.path.relpath(f, src)
    st, out, _ = call("POST", "/api/tool/write", {"path": rel, "content": open(f, encoding="utf-8", errors="replace").read()})
    assert st == 200, out
st, out, _ = call("POST", "/api/tool/commit", {"message": "load"}); assert st == 200, out
load_s = time.perf_counter() - t
big = max(files, key=os.path.getsize); bigrel = os.path.relpath(big, src)
def med(method, path, body=None, n=5):
    ts = []
    for _ in range(n):
        st, out, ms = call(method, path, body); assert st == 200, (path, out[:300]); ts.append(ms)
    return round(statistics.median(ts), 1), len(out)
r = {"files": len(files), "bytes": sum(map(os.path.getsize, files)), "load_s": round(load_s, 2), "big": os.path.getsize(big)}
r["tools"] = med("GET", "/api/tools")
r["search"] = med("GET", "/api/tool/search?query=ledger")
r["read_range"] = med("GET", f"/api/tool/read?path={bigrel}&start_line=100&end_line=120")
r["history"] = med("GET", "/api/tool/history?limit=20")
text = open(big, encoding="utf-8", errors="replace").read()
import collections
cnt = collections.Counter(text.split())
word = next(w for w, c in sorted(cnt.items(), key=lambda x: -len(x[0])) if c == 1 and w.isascii() and len(w) < 40 and text.count(w) == 1)
st, out, ms = call("POST", "/api/tool/edit", {"path": bigrel, "old": word, "new": word + "X"}); assert st == 200, out
r["edit_big_ms"] = round(ms, 1)
st, out, ms = call("POST", "/api/tool/commit", {"message": "agent edit"}); assert st == 200; r["commit_ms"] = round(ms, 1)
small = min(files, key=os.path.getsize); smallrel = os.path.relpath(small, src)
ts = []
for i in range(10):
    st, out, ms = call("POST", "/api/tool/write", {"path": smallrel, "content": f"v{i}\n"}); assert st == 200; ts.append(ms)
r["write_small_ms"] = round(statistics.median(ts), 1)
st, out, ms = call("POST", "/api/commit?message=small%20edits", None); assert st == 200; r["api_commit_ms"] = round(ms, 1)
hist = json.loads(call("GET", "/api/tool/history?limit=5")[1])["text"].splitlines()
ids = [l.split()[0] for l in hist if l.strip() and not l.startswith(" ")]
r["diff"] = med("GET", f"/api/tool/diff?from={ids[2]}&to={ids[1]}&path={bigrel}", n=3)
cyc = []
for i in range(3):
    t0 = time.perf_counter()
    assert call("POST", "/api/tool/draft_open", {"name": f"d{i}"})[0] == 200
    assert call("POST", "/api/tool/write", {"path": f"draft/{i}.md", "content": "x\n"})[0] == 200
    assert call("POST", "/api/tool/commit", {"message": "on draft"})[0] == 200
    assert call("GET", f"/api/tool/read?path=draft/{i}.md&at=main")[0] == 400
    st, out, _ = call("POST", "/api/tool/draft_promote", {}); assert st == 200, out
    assert call("GET", f"/api/tool/read?path=draft/{i}.md&at=main")[0] == 200
    cyc.append((time.perf_counter() - t0) * 1000)
r["draft_cycle_ms"] = round(statistics.median(cyc), 1)
r["get_write_refused"] = call("GET", "/api/tool/write?path=x&content=y")[0]
req = urllib.request.Request(base + "/api/tool/write", data=b'{"path":"x","content":"y"}', method="POST")
try: urllib.request.urlopen(req); r["no_header"] = 200
except urllib.error.HTTPError as e: r["no_header"] = e.code
p.terminate(); p.wait()
print(json.dumps(r))
