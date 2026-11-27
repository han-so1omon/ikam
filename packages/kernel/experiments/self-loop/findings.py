"""E401: the loop's findings graph, stored in IKAM and versioned by commit.

The graph is what the loop knows, in a form any loop (or person, or other
store) can read without the whole experiment log:

- facts/   one per measured candidate: the changes from the base commit,
           the parent it was built on, the measured scores and verdict, and
           the firing it came from. A fact never changes. A later finding
           that contradicts a kept one is a new fact with `contradicts` set,
           and the kept candidate is reopened (`reopen` firing), never
           edited.
- hints/   short state notes with the facts they rest on: a constant that
           does not move the north score at a commit, a change that alone
           makes it worse there, a catalog used up at a commit. Derived here from the facts; the owner may add his own
           as files in experiments/self-loop/hints/.
- intents/ candidates proposed and not yet measured, with the facts that
           motivated them.

Every file is named by a hash of its content, so two stores' graphs merge
by union (the same finding found twice is one file) and a graph can be
shared like any other tree: `ikam commit DIR --ref loop/findings`, read
back with `ikam checkout loop/findings DEST`.

    python3 -I findings.py build STORE DEST     # graph from the store's firings, committed
    python3 -I findings.py merge A B DEST       # union of two graphs
"""
import hashlib, json, os, shutil, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import fixed  # noqa: E402


def fid(kind, body):
    return kind[0] + hashlib.sha256(json.dumps(body, sort_keys=True).encode()).hexdigest()[:16]


def changes(rec):
    """The changes from the base commit a record stands for, sorted."""
    ch = rec.get("changes") or [{"name": rec["name"], "base": rec["base"], "value": rec["value"]}]
    return sorted(({"name": c["name"], "base": str(c["base"]), "value": str(c["value"])} for c in ch),
                  key=lambda c: c["name"])


def build(history, base_commits):
    """facts, hints, intents from a net's history (JSON lines of `ikam net
    history`). base_commits: round -> the commit its candidates were
    measured against."""
    out = {}
    finished = {}
    proposed = {}
    for f in history:
        if not f["output"]:
            continue
        o = json.loads(f["output"])
        if f["transition"] == "finish" and o.get("name") != "base":
            finished[f["token"]] = (f["key"], o)
        elif f["transition"] == "propose" and o.get("name") != "base":
            proposed[f["token"]] = o
        elif f["transition"] == "reopen":
            body = {"kind": "contradiction", "reopens": f["token"], "by": o["by"], "source": f["key"]}
            out[fid("fact", body)] = body
    facts = {}
    for tok, (key, o) in finished.items():
        body = {"kind": "measured", "round": o["round"], "changes": changes(o),
                "added": {"name": o["name"], "base": str(o["base"]), "value": str(o["value"])},
                "parent": o.get("parent"), "base_commit": base_commits.get(o["round"], o.get("base_commit")),
                "dn": o.get("dn"), "ddn": o.get("ddn"), "dn_full": o.get("dn_full", []),
                "dc_full": o.get("dc_full", []), "verdict": o["verdict"], "secs": o.get("secs"),
                "token": tok, "source": key}
        facts[fid("fact", body)] = body
    out.update(facts)
    # hints, each with the facts it rests on
    by_const = {}
    for i, f in facts.items():
        if f["base_commit"] and len(f["changes"]) == 1 and f["dn"] is not None:
            by_const.setdefault((f["base_commit"], f["added"]["name"]), []).append(i)
    for (commit, name), ids in sorted(by_const.items()):
        if all(abs(facts[i]["dn"]) < 1e-12 for i in ids):
            body = {"kind": "zero-move", "constant": name, "base_commit": commit, "from": sorted(ids),
                    "text": f"{name} does not move the north score at {commit[:12]}"}
            out[fid("hint", body)] = body
    for i, f in sorted(facts.items()):
        if f["base_commit"] and len(f["changes"]) == 1 and f["dn"] is not None and f["dn"] > fixed.TOL_N:
            a = f["added"]
            body = {"kind": "worse-alone", "constant": a["name"], "value": a["value"], "base_commit": f["base_commit"],
                    "from": [i], "text": f"{a['name']} = {a['value']} alone makes north worse at {f['base_commit'][:12]}"}
            out[fid("hint", body)] = body
    rounds = {}
    for i, f in facts.items():
        if f["base_commit"] and len(f["changes"]) == 1:
            rounds.setdefault((f["round"], f["base_commit"]), []).append(i)
    for (rnd, commit), ids in sorted(rounds.items()):
        if len(ids) >= 20 and not any(facts[i]["verdict"] == "keep" for i in ids):
            body = {"kind": "used-up", "round": rnd, "base_commit": commit, "from": sorted(ids),
                    "text": f"no one-constant change from the catalog is kept at {commit[:12]} ({len(ids)} tried)"}
            out[fid("hint", body)] = body
    for tok, o in proposed.items():
        if tok not in finished:
            body = {"kind": "intent", "token": tok, "changes": changes(o), "parent": o.get("parent"),
                    "motivated_by": o.get("motivated_by", [])}
            out[fid("intent", body)] = body
    return out


def write(graph, dest):
    if os.path.exists(dest):
        shutil.rmtree(dest)
    for i, body in graph.items():
        d = {"f": "facts", "h": "hints", "i": "intents"}[i[0]]
        os.makedirs(os.path.join(dest, d), exist_ok=True)
        with open(os.path.join(dest, d, i + ".json"), "w") as f:
            json.dump(body, f, sort_keys=True, indent=1)
            f.write("\n")


def read(dest):
    graph = {}
    for d in ("facts", "hints", "intents"):
        p = os.path.join(dest, d)
        for n in sorted(os.listdir(p)) if os.path.isdir(p) else []:
            graph[n[:-5]] = json.load(open(os.path.join(p, n)))
    return graph


def owner_hints():
    p = os.path.join(HERE, "hints")
    out = {}
    for n in sorted(os.listdir(p)) if os.path.isdir(p) else []:
        body = json.load(open(os.path.join(p, n)))
        body["kind"] = "owner"
        out[fid("hint", body)] = body
    return out


def commit(ikam, store, dest, msg):
    r = subprocess.run([ikam, "--store", store, "commit", dest, "--ref", "loop/findings", "-m", msg],
                       capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-1000:])
    return r.stdout.strip()


def checkout(ikam, store, dest):
    if os.path.exists(dest):
        shutil.rmtree(dest)
    r = subprocess.run([ikam, "--store", store, "checkout", "loop/findings", dest], capture_output=True, text=True)
    if r.returncode:
        return {}
    return read(dest)


def merge(a, b):
    """Union by content name; the same name with other content is refused."""
    out = dict(a)
    for k, v in b.items():
        if k in out and out[k] != v:
            raise SystemExit(f"merge: {k} differs between the graphs")
        out[k] = v
    return out


def news(graph, seen):
    """What a re-plan would be for (E401, re-plan on news only): kept or
    contradicting facts and hints not in `seen`."""
    return sorted(k for k, v in graph.items() if k not in seen and (
        k[0] == "h" or v.get("kind") == "contradiction" or v.get("verdict") == "keep"))


if __name__ == "__main__":
    if sys.argv[1] == "merge":
        write(merge(read(sys.argv[2]), read(sys.argv[3])), sys.argv[4])
