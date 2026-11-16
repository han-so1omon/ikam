"""E322: pilot runs against full runs of the fixed bench.

For each variant (base, then one tunable changed at a time), build the
bench, run the full command twice (base three times), then each corpus
alone as a pilot (base twice), recording each run's output and wall time.
Results go to target/pilot-runs (or $PILOT_RUNS). Run from anywhere:

    python3 -I experiments/pilot-runs/drive.py [VARIANT ...]

It edits and restores src/ and examples/ with git: commit or stash first,
and run nothing else on the machine meanwhile (timings are scored).
"""
import json, os, subprocess, sys, time
K = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
OUT = os.environ.get("PILOT_RUNS", os.path.join(K, "target", "pilot-runs"))
# repo-history, repo-graph, structured and transcripts need pinned commits that are not in the
# rewritten history (as for E265, E266): left out of "full" here.
CORPORA = ["md","pdf","office","invoices","synthetic-history","generated","multiformat",
           "north/cases","north/multiformat","north/tables"]
FULL = " ".join(CORPORA)
V = [
 ("base", None, None, None),
 ("base-again", None, None, None),
 ("store-level-15", "src/dict.rs", "const STORE_LEVEL: i32 = 19;", "const STORE_LEVEL: i32 = 15;"),
 ("store-level-22", "src/dict.rs", "const STORE_LEVEL: i32 = 19;", "const STORE_LEVEL: i32 = 22;"),
 ("measure-level-1", "src/dict.rs", "const MEASURE_LEVEL: i32 = 3;", "const MEASURE_LEVEL: i32 = 1;"),
 ("measure-level-6", "src/dict.rs", "const MEASURE_LEVEL: i32 = 3;", "const MEASURE_LEVEL: i32 = 6;"),
 ("star-16", "src/repack.rs", "const STAR_CANDIDATES: usize = 64;", "const STAR_CANDIDATES: usize = 16;"),
 ("star-256", "src/repack.rs", "const STAR_CANDIDATES: usize = 64;", "const STAR_CANDIDATES: usize = 256;"),
 ("gap-diff-256", "src/template.rs", "const GAP_DIFF: usize = 1024;", "const GAP_DIFF: usize = 256;"),
 ("gap-diff-4096", "src/template.rs", "const GAP_DIFF: usize = 1024;", "const GAP_DIFF: usize = 4096;"),
 ("dict-shortlist-3", "src/repack.rs", "const DICT_SHORTLIST: usize = 1;", "const DICT_SHORTLIST: usize = 3;"),
 ("min-match-48", "src/matcher.rs", "pub const MIN_MATCH: usize = 96;", "pub const MIN_MATCH: usize = 48;"),
 ("min-match-192", "src/matcher.rs", "pub const MIN_MATCH: usize = 96;", "pub const MIN_MATCH: usize = 192;"),
 ("match-candidates-16", "src/matcher.rs", "const MAX_CANDIDATES: usize = 4;", "const MAX_CANDIDATES: usize = 16;"),
 ("group-size-32k", "src/group.rs", "pub const GROUP_SIZE: usize = 64 * 1024;", "pub const GROUP_SIZE: usize = 32 * 1024;"),
 ("group-shortlist-2", "src/group.rs", "const SHORTLIST: usize = 8;", "const SHORTLIST: usize = 2;"),
 ("neighbors-1", "src/ingest.rs", "const NEIGHBORS: usize = 3;", "const NEIGHBORS: usize = 1;"),
 ("search-chunk-4k", "src/search.rs", "const MAX_CHUNK: usize = 8192;", "const MAX_CHUNK: usize = 4096;"),
 ("archive-reads-8", "examples/bench.rs", "const ARCHIVE_READS: f64 = 2.0;", "const ARCHIVE_READS: f64 = 8.0;"),
 ("graph-chunk-2k", "src/graph.rs", "const MIN_CHUNK: usize = 4096;", "const MIN_CHUNK: usize = 2048;"),
]
def sh(cmd, **kw):
    return subprocess.run(cmd, shell=True, cwd=K, capture_output=True, text=True, **kw)
def run_bench(args, path):
    t = time.time()
    r = sh(f"./target/release/examples/bench --json {args}")
    wall = time.time() - t
    with open(path, "w") as f: f.write(r.stdout)
    if r.returncode != 0:
        with open(path + ".err", "w") as f: f.write(r.stderr[-4000:])
    return wall, r.returncode
only = sys.argv[1:]
os.makedirs(OUT, exist_ok=True)
for name, file, old, new in V:
    if only and name not in only: continue
    d = f"{OUT}/{name}"; os.makedirs(d, exist_ok=True)
    if os.path.exists(d + "/done"): continue
    sh("git checkout -- src examples")
    if file:
        p = f"{K}/{file}"; s = open(p).read()
        assert s.count(old) == 1, (name, old)
        open(p, "w").write(s.replace(old, new))
    t = time.time(); b = sh("cargo build --release --example bench")
    meta = {"name": name, "build_s": round(time.time() - t, 1), "build_rc": b.returncode}
    if b.returncode != 0:
        meta["err"] = b.stderr[-2000:]; json.dump(meta, open(d + "/meta.json", "w")); continue
    fulls = 3 if name.startswith("base") else 2
    meta["full"] = []
    for i in range(fulls):
        w, rc = run_bench(FULL, f"{d}/full{i}.jsonl"); meta["full"].append([round(w, 1), rc])
    meta["pilot"] = {}
    reps = 2 if name.startswith("base") else 1
    for c in CORPORA:
        for i in range(reps):
            w, rc = run_bench(c, f"{d}/pilot-{c.replace('/','_')}-{i}.jsonl")
            meta["pilot"].setdefault(c, []).append([round(w, 2), rc])
    json.dump(meta, open(d + "/meta.json", "w"), indent=1)
    open(d + "/done", "w").write("ok")
    print(name, meta["build_s"], meta["full"], flush=True)
sh("git checkout -- src examples")
