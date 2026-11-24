"""E400, E401: a self-improvement loop for the kernel, run on IKAM.

The loop is configuration, not a feature of IKAM: its net is `self.net`, run
by the general `ikam net` command (one firing a call, history on a ref);
its steps are the modules `loop.json` names (candidates, parents, novelty,
rank); its scoring, keep rule and guards are `fixed.py`, which no config or
step can replace. What it has learned is the findings graph
(`findings.py`), committed to the ref `loop/findings` of the same store,
which the proposer reads instead of the experiment log and which another
store can merge.

A round: measure the base, fire `plan`, then until the round's test-hour
budget is spent pick a parent (steps/parents.py), list the changes that
could be added to it, drop repeats (steps/novelty.py), take the one the
policy ranks first, and measure it on its own branch by the fixed procedure
(`propose`, `start`, `finish`, `keep` or `close`). Kept candidates wait for
the owner's merge, a human transition `ikam net` never fires; a kept
candidate that a later finding contradicts is reopened (`reopen`), never
edited. After a round, a meta step proposes new ranking policies and adopts
one only if it ranks held-out rounds better (fixed.kept_per_hour); with
`"replan": "news"` it runs only when the graph has news (a kept or
contradicting fact, or a hint) since the last one.

    python3 -I loop.py migrate STORE HISTORY_JSONL   # replay an exported history
    python3 -I loop.py round STORE WORKTREE R         # measure round R
    python3 -I loop.py meta  STORE                    # one meta step
    python3 -I loop.py graph STORE                    # rebuild and commit the findings graph
    python3 -I loop.py score STORE R                  # kept per test-hour of round R
    python3 -I loop.py check STORE

WORKTREE is a separate git worktree at the base commit; candidates are
committed there on exp/400-cand-* branches. Nothing is pushed or merged.
"""
import json, os, random, statistics as st, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
K = os.path.abspath(os.path.join(HERE, "..", ".."))
ROOT = os.path.abspath(os.path.join(K, "..", ".."))
sys.path.insert(0, HERE)
import importlib  # noqa: E402

import findings, fixed  # noqa: E402
from steps import novelty, parents  # noqa: E402

IKAM = os.path.join(K, "target", "release", "ikam")
NET = os.path.join(HERE, "self.net")
CFG = json.load(open(os.path.join(HERE, "loop.json")))
fixed.check_config(CFG)
catalog = importlib.import_module("steps." + CFG["steps"]["candidates"])  # where candidates come from
policy = importlib.import_module(CFG["steps"]["rank"])  # how they are ranked
FIRST = {"weights": {k: 0.0 for k in policy.FEATURES}, "sigma": 1.0, "seed": 400}


def sh(cmd, cwd, check=True):
    r = subprocess.run(cmd, cwd=cwd, shell=isinstance(cmd, str), capture_output=True, text=True)
    if check and r.returncode:
        raise SystemExit(f"{cmd}: {r.stderr[-2000:]}")
    return r


def net(store, *a):
    return sh([IKAM, "--store", store, "net", NET, "--ref", CFG["ref"], *a], K)


def fire(store, t, tok, out=None):
    args = ["fire", t, tok]
    if out is not None:
        os.makedirs(store + ".out", exist_ok=True)
        p = os.path.join(store + ".out", f"{t}-{tok}.json")
        with open(p, "w") as f:
            json.dump(out, f, sort_keys=True)
        args.append(p)
    net(store, *args)


def history(store):
    if not os.path.exists(store):
        return []
    return [json.loads(l) for l in net(store, "history").stdout.splitlines()]


def outputs(h, t):
    return {f["token"]: json.loads(f["output"]) for f in h if f["transition"] == t and f["output"]}


def records(store, h=None):
    """Measured candidates by round, from the finish firings."""
    rounds = {}
    for r in outputs(h if h is not None else history(store), "finish").values():
        if r.get("name") != "base":
            rounds.setdefault(r["round"], []).append(r)
    return rounds


def current(store, h=None):
    """The policy in use (last adopted) with the meta step's latest radius."""
    h = h if h is not None else history(store)
    out = outputs(h, "variant")
    adopted = [f["token"] for f in h if f["transition"] == "adopt"]
    if not adopted:
        return None, None
    p = dict(out[adopted[-1]]["next"])
    p["sigma"] = out[[f["token"] for f in h if f["transition"] == "variant"][-1]]["next"]["sigma"]
    return adopted[-1], p


def base_commits(h):
    return {o["round"]: o["commit"] for t, o in outputs(h, "propose").items() if t.endswith("-base")}


def graph(store, h=None, commit=True):
    """The findings graph from the store's firings (and the owner's hints);
    committed to loop/findings and read back from it, so the loop reads
    what IKAM holds."""
    h = h if h is not None else history(store)
    g = findings.build(h, base_commits(h))
    g.update(findings.owner_hints())
    if not commit:
        return g
    findings.write(g, store + ".findings")
    findings.commit(IKAM, store, store + ".findings", f"findings: {len(g)} entries")
    back = findings.checkout(IKAM, store, store + ".findings.read")
    assert back == g, "the findings graph read back from IKAM differs"
    return back


def hist_from(g):
    """The policy's history: every measured fact, by the change it added
    (its effect on its parent where it had one)."""
    out = []
    for f in g.values():
        if f.get("kind") == "measured":
            dn = f["ddn"] if f.get("ddn") is not None else f["dn"]
            out.append(dict(name=f["added"]["name"], base=f["added"]["base"], value=f["added"]["value"],
                            dn=dn, secs=f["secs"], verdict=f["verdict"]))
    return out


def migrate(store, path):
    """Replay an exported history (E400's) into this net, firing for firing."""
    for line in open(path):
        f = json.loads(line)
        fire(store, f["transition"], f["token"], json.loads(f["output"]) if f["output"] else None)
    graph(store)


def meta(store):
    h = history(store)
    tok, p = current(store, h)
    p = p or FIRST
    g = graph(store, h)
    last = list(outputs(h, "variant").values())
    seen = set(last[-1].get("seen", [])) if last else set()
    new = findings.news(g, seen)
    if CFG.get("replan") == "news" and last and "seen" in last[-1] and not new:
        print("no news since the last meta step: policy kept, nothing fired")
        return False
    rounds = records(store, h)
    nxt, adopt, cur, best = policy.improve(p, rounds)
    name = f"p{len(last)}"
    fire(store, "variant", name, {"from": tok, "next": nxt, "held_out_before": round(cur, 4),
                                  "held_out_best": round(best, 4), "rounds": sorted(rounds),
                                  "seen": sorted(g), "news": new})
    if adopt or tok is None:
        if tok:
            fire(store, "retire", tok)
        fire(store, "adopt", name)
    else:
        fire(store, "reject", name)
    print(f"{name}: held-out area {cur:.3f} -> best variant {best:.3f}: {'adopted' if adopt or tok is None else 'rejected'}; "
          f"sigma {nxt['sigma']:.2f}; news {len(new)}")
    return True


def timed(cmd, cwd):
    t = time.time()
    r = sh(cmd, cwd, check=False)
    return r, time.time() - t


def bench(wt, args):
    r, s = timed(f"./target/release/examples/bench --json {' '.join(args)}", os.path.join(wt, "packages/kernel"))
    return r.returncode, fixed.scores(r.stdout), s


def measure(wt, c, base_commit, base, tol_c, pins, branch):
    """Build c (its parent's commit plus one catalog line) on its own branch
    and run the fixed procedure on it."""
    k = os.path.join(wt, "packages/kernel")
    row, value, start = c["row"], c["value"], c["parent_commit"] or base_commit
    fixed.check_branch(branch)
    if start != base_commit:
        fixed.check_chain(wt, base_commit, start, catalog.rows())
    sh(["git", "checkout", "-q", "-B", branch, start], wt)
    p = os.path.join(k, row["file"])
    s = open(p).read()
    assert s.count(row["line"]) == 1, row["line"]
    open(p, "w").write(s.replace(row["line"], row["line"].replace(f"= {row['value']};", f"= {value};")))
    fixed.check_diff(wt, start, row, value)
    on = f" on {c['parent']}" if c["parent"] else ""
    sh(["git", "commit", "-qam", f"E400 candidate: {row['name']} {row['value']} -> {value}{on}\n\n"
        "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"], wt)
    rec = dict(name=row["name"], base=row["value"], value=value, branch=branch,
               commit=sh(["git", "rev-parse", "HEAD"], wt).stdout.strip(),
               parent=c["parent"], parent_commit=c["parent_commit"], changes=c["changes"])
    b, secs = timed("cargo build --release --example bench", k)
    rec.update(dn=None, ddn=None, dn_full=[], dc_full=[], gates=[])
    if b.returncode:
        rec.update(verdict="gate-failed", gates=["build"], secs=round(secs))
        return rec
    rc, (_, north), s = bench(wt, fixed.NORTH)
    secs += s
    if rc or north is None:
        rec.update(verdict="gate-failed", gates=["bench north"], secs=round(secs))
        return rec
    rec["dn"] = north / base["north"] - 1
    rec["ddn"] = rec["dn"] - (c["parent_dn"] or 0.0)
    if rec["dn"] < -fixed.TOL_N:
        for _ in range(2):
            rc, (cost, n2), s = bench(wt, fixed.FULL)
            secs += s
            if rc:
                rec["gates"].append("bench full")
                break
            rec["dc_full"].append(cost / base["cost"] - 1)
            rec["dn_full"].append(n2 / base["north"] - 1)
    rec["verdict"] = "gate-failed" if rec["gates"] else fixed.verdict(rec["dn"], rec["dn_full"], rec["dc_full"], tol_c)
    if rec["verdict"] == "keep":
        t, s = timed("cargo test --release -q", k)
        secs += s
        if t.returncode:
            # E401: keep which test failed, so a later round (or the owner)
            # can tell a broken change from a flaky test without rerunning.
            fails = [l for l in (t.stdout + t.stderr).splitlines() if "FAILED" in l or "panicked" in l]
            rec.update(verdict="gate-failed", gates=["cargo test"], gate_tail=fails[-8:])
    if fixed.manifest(wt) != pins["wt"] or fixed.manifest(ROOT) != pins["loop"]:
        raise SystemExit("guard: a protected file changed during the round")
    rec["secs"] = round(secs)
    return rec


def options(g, parent, base_commit, rows, commits):
    """Every child of parent the catalog allows (one more constant, not one
    the parent already changes), split by the novelty step's verdict."""
    have = {c["name"] for c in (parent["changes"] if parent else [])}
    out, skipped = [], []
    for ch in rows:
        if ch["name"] in have:
            continue
        changes = sorted((parent["changes"] if parent else []) +
                         [{"name": ch["name"], "base": ch["base"], "value": ch["value"]}], key=lambda x: x["name"])
        why = novelty.repeat(CFG, g, base_commit, changes, {"name": ch["name"], "value": ch["value"]})
        c = dict(ch, changes=changes, parent=parent["token"] if parent else None,
                 parent_commit=commits[parent["token"]] if parent else None,
                 parent_dn=parent["dn"] if parent else 0.0)
        if why:
            skipped.append({"changes": changes, "why": why})
        else:
            out.append(c)
    return out, skipped


def slug(c):
    s = f"{c['name']}-{policy.num(c['value'])}"
    if c["parent"]:
        s += "-on-" + c["parent"]
    return s if len(s) <= 100 else s[:80] + "-" + findings.fid("x", c["changes"])[1:9]


def round_(store, wt, rnd):
    """Measure round rnd. A killed round resumes from the store: every step
    already fired is read back instead of measured again."""
    h = history(store)
    tok, p = current(store, h)
    assert p, "migrate first"
    fired = {f["key"] for f in h}
    out = {(f["transition"], f["token"]): json.loads(f["output"]) for f in h if f["output"]}
    started = [o.get("policy") for (t, _), o in out.items() if t == "propose" and o.get("round") == rnd]
    started = [x for x in started if x]
    if started and started[0] != tok:  # a resumed round keeps the policy it started with
        tok, p = started[0], out[("variant", started[0])]["next"]

    def once(t, token, o=None):
        if f"{t}-{token}" not in fired:
            fire(store, t, token, o)
            fired.add(f"{t}-{token}")

    pins = {"wt": fixed.manifest(wt), "loop": fixed.manifest(ROOT)}
    k = os.path.join(wt, "packages/kernel")
    b = f"{rnd}-base"
    if ("finish", b) in out:
        base_commit, base = out[("propose", b)]["commit"], out[("finish", b)]
        tol_c = base["tol_c"]
        if base["pins"] != pins:
            raise SystemExit("guard: protected files differ from the round's start")
        sh(["git", "checkout", "-q", "--detach", base_commit], wt)
        print(f"{b}: resumed from the store", flush=True)
    else:
        base_commit = sh(["git", "rev-parse", "HEAD"], wt).stdout.strip()
        sh("cargo build --release --example bench", k)
        _, (_, north), _ = bench(wt, fixed.NORTH)
        costs = [bench(wt, fixed.FULL)[1][0] for _ in range(3)]
        base = {"north": north, "cost": st.median(costs), "costs": costs}
        tol_c = fixed.cost_tol(costs)
        once("propose", b, {"round": rnd, "name": "base", "commit": base_commit, "policy": tok})
        once("start", b)
        fire(store, "finish", b, dict(base, round=rnd, name="base", tol_c=tol_c, pins=pins))
        print(f"{b}: north {north:.5f}, cost {base['cost']:.4f} ({costs}), cost tolerance {tol_c:.3f}", flush=True)
    once("close", b)
    h = history(store)
    g = graph(store, h)
    commits = {t: o["commit"] for t, o in outputs(h, "finish").items() if o.get("commit")}
    once("plan", rnd, {"round": rnd, "base_commit": base_commit, "policy": tok, "steps": CFG["steps"],
                       "budget_hours": CFG["budget_hours"], "graph": sorted(g),
                       "reason": "a new round: nothing open to try"})
    rows = catalog.changes(CFG)
    rng = random.Random(f"{CFG['seed']}/{rnd}")
    spent = sum(r["secs"] for r in records(store, h).get(rnd, []))
    exhausted = set()
    while spent < CFG["budget_hours"] * 3600:
        try:
            parent = parents.pick(CFG, g, base_commit, rng, exhausted)
        except StopIteration:
            print("nothing left to try", flush=True)
            break
        opts, skipped = options(g, parent, base_commit, rows, commits)
        if not opts:
            exhausted.add(parent["token"] if parent else None)
            continue
        c = policy.rank(opts, hist_from(g), p)[0]
        t = f"{rnd}-{slug(c)}"
        motivated = sorted(i for i, f in g.items() if c["parent"] and f.get("token") == c["parent"]) + \
            sorted(i for i, f in g.items() if f.get("kind") in ("zero-move", "worse-alone", "used-up", "owner")
                   and f.get("base_commit") in (None, base_commit))
        once("propose", t, {"round": rnd, "name": c["name"], "base": c["base"], "value": c["value"],
                            "parent": c["parent"], "parent_commit": c["parent_commit"], "changes": c["changes"],
                            "policy": tok, "options": len(opts), "skipped": skipped, "motivated_by": motivated})
        once("start", t)
        if ("finish", t) in out:
            rec = out[("finish", t)]
        else:
            rec = measure(wt, c, base_commit, base, tol_c, pins, f"exp/400-cand-{t}")
            rec.update(round=rnd)
            fire(store, "finish", t, rec)
            fired.add(f"finish-{t}")
        once("keep" if rec["verdict"] == "keep" else "close", t)
        commits[t] = rec["commit"]
        spent += rec["secs"]
        on = f" on {c['parent']}" if c["parent"] else ""
        print(f"{t}: north {100 * (rec['dn'] or 0):+.2f}% ({100 * (rec['ddn'] or 0):+.2f}% on its parent) "
              f"{rec['verdict']} ({rec['secs']} s){on}; {len(skipped)} skipped as repeats", flush=True)
        h = history(store)
        g = graph(store, h, commit=False)
    sh(["git", "checkout", "-q", "--detach", base_commit], wt)
    reopen(store)
    graph(store)


def reopen(store):
    """Reopen every kept candidate that a later finding contradicts
    (fixed.contradicts): a new linked fact, never an edit."""
    h = history(store)
    g = graph(store, h, commit=False)
    facts = {f["token"]: f for f in g.values() if f.get("kind") == "measured"}
    done = {f["token"] for f in h if f["transition"] in ("reopen", "merge")}
    for tok in [f["token"] for f in h if f["transition"] == "keep"]:
        if tok in done or tok not in facts:
            continue
        keep = dict(facts[tok]["added"], verdict="keep")
        for i, later in sorted(g.items()):
            if later.get("kind") == "measured" and later["token"] != tok and \
                    fixed.contradicts(keep, dict(later["added"], verdict=later["verdict"])):
                fire(store, "reopen", tok, {"by": i})
                print(f"reopened {tok}: contradicted by {later['token']}")
                break


def score(store, rnd):
    recs = records(store)[rnd]
    out = {i: r for i, r in enumerate(recs)}
    kept = sum(r["verdict"] == "keep" for r in recs)
    hours = sum(r["secs"] for r in recs) / 3600
    res = {"measured": len(recs), "kept": kept, "hours": round(hours, 3),
           "kept_per_hour": round(kept / hours, 3) if hours else 0.0,
           "verdicts": {v: sum(r["verdict"] == v for r in recs) for v in sorted({r["verdict"] for r in recs})}}
    if all("fixed_rank" in r for r in recs):
        by = lambda key: [i for i, _ in sorted(out.items(), key=lambda x: x[1][key])]
        res["policy"] = fixed.kept_per_hour(by("policy_rank"), out)
        res["fixed"] = fixed.kept_per_hour(by("fixed_rank"), out)
        rng = random.Random(400)
        rs = [fixed.kept_per_hour(rng.sample(list(out), len(out)), out) for _ in range(2000)]
        res["random"] = {k: st.mean(r[k] for r in rs) for k in ("kept", "hours", "per_hour", "auc")}
    print(json.dumps(res, indent=1))
    return res


def check(store):
    print(net(store, "check").stdout.strip())
    m = json.loads(net(store, "marking").stdout)
    assert m["policy"] <= 1, "more than one policy in use"
    g = graph(store, commit=False)
    back = findings.checkout(IKAM, store, store + ".findings.read")
    assert back == g, "the committed findings graph is not the store's"
    kinds = {}
    for v in g.values():
        kinds[v.get("kind")] = kinds.get(v.get("kind"), 0) + 1
    print(f"loop/findings: {len(g)} entries, equal to the firings; {dict(sorted(kinds.items()))}")


if __name__ == "__main__":
    cmd, store = sys.argv[1], os.path.abspath(sys.argv[2])
    {"migrate": lambda: migrate(store, sys.argv[3]), "meta": lambda: meta(store), "graph": lambda: graph(store),
     "round": lambda: round_(store, os.path.abspath(sys.argv[3]), sys.argv[4]),
     "score": lambda: score(store, sys.argv[3]), "check": lambda: check(store)}[cmd]()
