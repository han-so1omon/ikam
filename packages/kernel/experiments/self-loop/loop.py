"""E400: a self-improvement loop for the kernel, with its state on IKAM.

Two levels. A round takes candidates (one catalog constant changed, each on
its own branch), ranks them by the current policy, measures them by the fixed
procedure and keeps or closes each. A meta step then proposes new policies
and adopts one only if it ranks held-out rounds better (fixed.kept_per_hour),
and adjusts its own search radius. Every step is a firing of the `self` net
on an IKAM store (examples/self_loop.rs), whose output is that step's record;
the loop reads its history back from there and keeps nothing else.

    python3 -I loop.py seed  STORE              # seed history + first policy
    python3 -I loop.py round STORE WORKTREE R   # measure a round R
    python3 -I loop.py meta  STORE              # one meta step on all rounds
    python3 -I loop.py score STORE R            # policy vs fixed vs random on R

WORKTREE is a separate git worktree at the base commit; candidates are
committed there on exp/400-cand-* branches. Nothing is pushed or merged here.
"""
import json, os, random, statistics as st, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
K = os.path.abspath(os.path.join(HERE, "..", ".."))
ROOT = os.path.abspath(os.path.join(K, "..", ".."))
sys.path.insert(0, HERE)
import fixed, policy  # noqa: E402

BIN = os.path.join(K, "target", "release", "examples", "self_loop")
MODEL = {"build": 30, "north": 104, "full": 230}  # seconds, measured on this machine (E400)
FIRST = {"weights": {k: 0.0 for k in policy.FEATURES}, "sigma": 1.0, "seed": 400}


def sh(cmd, cwd, check=True):
    r = subprocess.run(cmd, cwd=cwd, shell=isinstance(cmd, str), capture_output=True, text=True)
    if check and r.returncode:
        raise SystemExit(f"{cmd}: {r.stderr[-2000:]}")
    return r


def fire(store, t, tok, out=None):
    args = [BIN, store, "fire", t, tok]
    if out is not None:
        os.makedirs(store + ".out", exist_ok=True)
        p = os.path.join(store + ".out", f"{t}-{tok}.json")
        with open(p, "w") as f:
            json.dump(out, f, sort_keys=True)
        args.append(p)
    sh(args, K)


def history(store):
    if not os.path.exists(store):
        return []
    return [json.loads(l) for l in sh([BIN, store, "history"], K).stdout.splitlines()]


def records(store):
    """Measured candidates by round, from the finish firings."""
    rounds = {}
    for f in history(store):
        if f["transition"] == "finish" and f["output"]:
            r = json.loads(f["output"])
            if r.get("name") != "base":
                rounds.setdefault(r["round"], []).append(r)
    return rounds


def current(store):
    """The policy in use (last adopted) with the meta step's latest radius."""
    h = history(store)
    out = {f["token"]: json.loads(f["output"]) for f in h if f["transition"] == "variant"}
    adopted = [f["token"] for f in h if f["transition"] == "adopt"]
    if not adopted:
        return None, None
    p = dict(out[adopted[-1]]["next"])
    p["sigma"] = out[[f["token"] for f in h if f["transition"] == "variant"][-1]]["next"]["sigma"]
    return adopted[-1], p


def candidate(store, tok, rec, keep):
    fire(store, "propose", tok, {k: rec[k] for k in ("round", "name", "base", "value")})
    fire(store, "start", tok)
    fire(store, "finish", tok, rec)
    fire(store, "keep" if keep else "close", tok)


def seed(store):
    for line in open(os.path.join(HERE, "seed.tsv")):
        if line.startswith("#"):
            continue
        rnd, name, base, value, dn, v = line.rstrip("\n").split("\t")
        dn = float(dn) / 100
        secs = MODEL["build"] + MODEL["north"] + (2 * MODEL["full"] if dn < -fixed.TOL_N else 0)
        rec = dict(round=rnd, name=name, base=base, value=value, dn=dn, verdict=v, secs=secs,
                   source="docs/experiments/log.md " + rnd, secs_modelled=True)
        candidate(store, f"{rnd.lower()}-{name}-{value}", rec, v == "keep")
    meta(store)


def meta(store):
    tok, p = current(store)
    p = p or FIRST
    rounds = records(store)
    nxt, adopt, cur, best = policy.improve(p, rounds)
    n = sum(f["transition"] == "variant" for f in history(store))
    new = f"p{n}"
    fire(store, "variant", new, {"from": tok, "next": nxt, "held_out_before": round(cur, 4),
                                 "held_out_best": round(best, 4), "rounds": sorted(rounds)})
    if adopt or tok is None:
        if tok:
            fire(store, "retire", tok)
        fire(store, "adopt", new)
    else:
        fire(store, "reject", new)
    print(f"{new}: held-out area {cur:.3f} -> best variant {best:.3f}: {'adopted' if adopt or tok is None else 'rejected'}; sigma {nxt['sigma']:.2f}")


def catalog():
    rows = []
    for line in open(os.path.join(HERE, "catalog.tsv")):
        if not line.startswith("#"):
            name, file, l, value, tries = line.rstrip("\n").split("\t")
            rows.append(dict(name=name, file=file, line=l, value=value, tries=tries.split(",")))
    return rows


def timed(cmd, cwd):
    t = time.time()
    r = sh(cmd, cwd, check=False)
    return r, time.time() - t


def bench(wt, args):
    r, s = timed(f"./target/release/examples/bench --json {' '.join(args)}", os.path.join(wt, "packages/kernel"))
    return r.returncode, fixed.scores(r.stdout), s


def measure(wt, row, value, base_commit, base, tol_c, pins):
    k = os.path.join(wt, "packages/kernel")
    slug = f"{row['name']}-{policy.num(value)}"
    br = f"exp/400-cand-{slug}"
    fixed.check_branch(br)
    sh(["git", "checkout", "-q", "-B", br, base_commit], wt)
    p = os.path.join(k, row["file"])
    s = open(p).read()
    assert s.count(row["line"]) == 1, row["line"]
    open(p, "w").write(s.replace(row["line"], row["line"].replace(f"= {row['value']};", f"= {value};")))
    fixed.check_diff(wt, base_commit, row, value)
    sh(["git", "commit", "-qam", f"E400 candidate: {row['name']} {row['value']} -> {value}\n\n"
        "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"], wt)
    rec = dict(name=row["name"], base=row["value"], value=value, branch=br, commit=sh(["git", "rev-parse", "HEAD"], wt).stdout.strip())
    b, secs = timed("cargo build --release --example bench", k)
    rec.update(dn=None, dn_full=[], dc_full=[], gates=[])
    if b.returncode:
        rec.update(verdict="gate-failed", gates=["build"], secs=round(secs))
        return rec
    rc, (_, north), s = bench(wt, fixed.NORTH)
    secs += s
    if rc or north is None:
        rec.update(verdict="gate-failed", gates=["bench north"], secs=round(secs))
        return rec
    rec["dn"] = north / base["north"] - 1
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
            rec.update(verdict="gate-failed", gates=["cargo test"])
    if fixed.manifest(wt) != pins["wt"] or fixed.manifest(ROOT) != pins["loop"]:
        raise SystemExit("guard: a protected file changed during the round")
    rec["secs"] = round(secs)
    return rec


def round_(store, wt, rnd):
    tok, p = current(store)
    assert p, "run seed first"
    hist = [x for xs in records(store).values() for x in xs]
    pins = {"wt": fixed.manifest(wt), "loop": fixed.manifest(ROOT)}
    base_commit = sh(["git", "rev-parse", "HEAD"], wt).stdout.strip()
    k = os.path.join(wt, "packages/kernel")
    sh("cargo build --release --example bench", k)
    _, (_, north), _ = bench(wt, fixed.NORTH)
    costs = [bench(wt, fixed.FULL)[1][0] for _ in range(3)]
    base = {"north": north, "cost": st.median(costs), "costs": costs}
    tol_c = fixed.cost_tol(costs)
    fire(store, "propose", f"{rnd}-base", {"round": rnd, "name": "base", "commit": base_commit})
    fire(store, "start", f"{rnd}-base")
    fire(store, "finish", f"{rnd}-base", dict(base, round=rnd, name="base", tol_c=tol_c, pins=pins))
    fire(store, "close", f"{rnd}-base")
    pool = [dict(name=r["name"], base=r["value"], value=v, row=r) for r in catalog() for v in r["tries"]]
    fixed_order = [f"{c['name']}={c['value']}" for c in pool]
    order = policy.rank(pool, hist, p)
    for i, c in enumerate(order):
        t = f"{rnd}-{c['name']}-{policy.num(c['value'])}"
        fire(store, "propose", t, {"round": rnd, "name": c["name"], "base": c["base"], "value": c["value"],
                                   "policy": tok, "policy_rank": i, "fixed_rank": fixed_order.index(f"{c['name']}={c['value']}")})
        fire(store, "start", t)
        rec = measure(wt, c["row"], c["value"], base_commit, base, tol_c, pins)
        rec.update(round=rnd, policy_rank=i, fixed_rank=fixed_order.index(f"{c['name']}={c['value']}"))
        fire(store, "finish", t, rec)
        fire(store, "keep" if rec["verdict"] == "keep" else "close", t)
        print(f"{t}: north {100 * (rec['dn'] or 0):+.2f}% {rec['verdict']} ({rec['secs']} s)", flush=True)
    sh(["git", "checkout", "-q", "--detach", base_commit], wt)


def score(store, rnd):
    recs = records(store)[rnd]
    out = {i: r for i, r in enumerate(recs)}
    by = lambda key: [i for i, _ in sorted(out.items(), key=lambda x: x[1][key])]
    res = {"policy": fixed.kept_per_hour(by("policy_rank"), out), "fixed": fixed.kept_per_hour(by("fixed_rank"), out)}
    rng = random.Random(400)
    rs = [fixed.kept_per_hour(rng.sample(list(out), len(out)), out) for _ in range(2000)]
    res["random"] = {k: st.mean(r[k] for r in rs) for k in ("kept", "hours", "per_hour", "auc")}
    print(json.dumps(res, indent=1))
    return res


if __name__ == "__main__":
    cmd, store = sys.argv[1], os.path.abspath(sys.argv[2])
    {"seed": lambda: seed(store), "meta": lambda: meta(store),
     "round": lambda: round_(store, os.path.abspath(sys.argv[3]), sys.argv[4]),
     "score": lambda: score(store, sys.argv[3])}[cmd]()
