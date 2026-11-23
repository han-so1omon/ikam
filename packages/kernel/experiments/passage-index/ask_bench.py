"""E378: does an index of words make `ikam live` answer questions faster than
scanning all extracted text, with the same answers, and at what memory cost?

Serves STORE with `ikam live` once per flag set, records the time until it
answers and its resident memory (VmRSS, Linux /proc), then asks each question
REPS times through /api/ask (no model: sources only) and keeps the server's
own retrieval time. The sources of the first answer to each question are
compared across flag sets. 127.0.0.1 only.
  python3 -I ask_bench.py IKAM_BIN STORE REPS "FLAGS;FLAGS" """
import base64, json, os, shutil, socket, statistics, subprocess, sys, time
import urllib.parse, urllib.request

binary, store, reps, sets = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4].split(";")
PW = "e378-test"
QUESTIONS = [
    "read csv with a dtype per column", "groupby aggregate several functions",
    "merge two frames on the index", "datetime with a timezone", "parquet engine",
    "rolling window mean", "fill missing values", "copy on write", "string accessor",
    "plotting backend", "click option with a default", "http session retries",
    "quarterly revenue", "slide title", "table of contents", "deprecation warning",
    "categorical dtype ordered", "sparse array", "read excel sheet name", "json normalize",
]
auth = subprocess.run([binary, "passwd", "u"], input=PW + "\n", capture_output=True,
                      text=True, check=True).stdout.strip().split("=", 1)[1]


def free_port():
    s = socket.socket(); s.bind(("127.0.0.1", 0)); p = s.getsockname()[1]; s.close(); return p


def get(port, path, timeout=600):
    r = urllib.request.Request("http://127.0.0.1:%d%s" % (port, path))
    r.add_header("Authorization", "Basic " + base64.b64encode(("u:" + PW).encode()).decode())
    with urllib.request.urlopen(r, timeout=timeout) as resp:
        return resp.read()


def rss(pid):
    for line in open("/proc/%d/status" % pid):
        if line.startswith("VmRSS"):
            return round(int(line.split()[1]) / 1024)


answers = {}
for fl in sets:
    copy = store + ".run"
    shutil.rmtree(copy, ignore_errors=True); shutil.copytree(store, copy)
    port = free_port()
    t0 = time.time()
    p = subprocess.Popen([binary, "--store", copy, "live", "--commit-after", "600000"]
                         + fl.split() + [str(port)], env=dict(os.environ, IKAM_AUTH=auth),
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    while True:
        try:
            get(port, "/api/status", 5); break
        except Exception:
            if p.poll() is not None:
                raise SystemExit("server exited")
            time.sleep(0.2)
    ready = time.time() - t0
    mem_ready = rss(p.pid)
    ms, first = [], {}
    for q in QUESTIONS:
        for i in range(reps):
            j = json.loads(get(port, "/api/ask?q=" + urllib.parse.quote(q)))
            ms.append(j["ms"])
            if i == 0:
                first[q] = [(s["path"], s["part"], s["line"], s["score"]) for s in j["sources"]]
    answers[fl] = first
    ms.sort()
    print(json.dumps({"flags": fl, "ready_s": round(ready, 1), "rss_mb": mem_ready,
                      "rss_after_mb": rss(p.pid), "asks": len(ms),
                      "ms_p50": round(statistics.median(ms), 2),
                      "ms_p95": round(ms[int(len(ms) * 0.95) - 1], 2),
                      "ms_max": round(ms[-1], 2),
                      "empty_answers": sum(1 for v in first.values() if not v)}), flush=True)
    p.terminate(); p.wait()
    shutil.rmtree(copy, ignore_errors=True)
base = answers[sets[0]]
for fl in sets[1:]:
    same = sum(answers[fl][q] == base[q] for q in QUESTIONS)
    print(json.dumps({"same_answers": "%d/%d" % (same, len(QUESTIONS)), "vs": sets[0], "flags": fl}))
