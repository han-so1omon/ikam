"""E400: the ranking policy, the part of the loop that the loop changes.

A policy is a weight per feature of a candidate, read from the loop's own
history (every measured candidate so far), plus the meta step's own search
radius. `rank` orders a pool by it; `improve` proposes variants and judges
each with fixed.kept_per_hour on rounds it did not see (leave one round
out), so a policy is only adopted for doing better on held-out history.
The evaluator, the keep rule and the tests are in fixed.py, out of reach.
"""
import hashlib, math, random

import fixed

FEATURES = ["moves", "better", "worse", "toward", "step", "untested", "hours", "parent"]


def num(s):
    return math.prod(int(x) for x in str(s).split("*"))


def features(c, hist):
    """c: {name, base, value}; hist: earlier records {name, base, value, dn, secs}."""
    h = [r for r in hist if r["name"] == c["name"] and r.get("dn") is not None]
    b, v = num(c["base"]), num(c["value"])
    good = [num(r["value"]) for r in h if r["dn"] < -fixed.TOL_N]
    secs = [r["secs"] for r in hist if r.get("secs")] or [3600]
    own = [r["secs"] for r in h if r.get("secs")] or secs
    return {
        "moves": min(3.0, sum(abs(r["dn"]) for r in h) / len(h) / 0.01) if h else 0.0,
        "better": (sum(r["dn"] < -fixed.TOL_N for r in h) + 0.5) / (len(h) + 1),
        "worse": (sum(r["dn"] > fixed.TOL_N for r in h) + 0.5) / (len(h) + 1),
        "toward": float(any((g - b) * (v - b) > 0 for g in good)),
        "step": abs(math.log2(v / b)) if b > 0 and v > 0 else 1.0,
        "untested": float(not h),
        "hours": (sum(own) / len(own)) / 3600,
        # E401: how much the candidate's parent improved north (0 on the base)
        "parent": -float(c.get("parent_dn") or 0.0) / 0.01,
    }


def tie(seed, c):
    return hashlib.sha256(f"{seed}/{c['name']}/{c['value']}".encode()).hexdigest()


def rank(pool, hist, p):
    w = p["weights"]

    def key(c):
        f = features(c, hist)
        return (-sum(w.get(k, 0.0) * f[k] for k in FEATURES), tie(p["seed"], c))

    return sorted(pool, key=key)


def held_out(p, rounds):
    """Mean area score over rounds with a kept change, each ranked from the
    other rounds' records only."""
    scores = []
    for r, recs in rounds.items():
        out = {i: x for i, x in enumerate(recs)}
        if not any(x["verdict"] == "keep" for x in recs):
            continue
        hist = [x for q, xs in rounds.items() if q != r for x in xs]
        order = rank([dict(x, _i=i) for i, x in out.items()], hist, p)
        scores.append(fixed.kept_per_hour([x["_i"] for x in order], out)["auc"])
    return sum(scores) / len(scores) if scores else 0.0


def improve(p, rounds, n=64):
    """One meta step: n variants around p at radius sigma. The best is
    adopted when it beats p on held-out rounds by more than 0.01; the radius
    grows after an adoption and shrinks after a rejection (the meta step
    tuning its own search)."""
    rng = random.Random(f"{p['seed']}/{len(rounds)}/{sum(len(v) for v in rounds.values())}")
    cur = held_out(p, rounds)
    best, best_s = None, -1.0
    for _ in range(n):
        w = {k: p["weights"].get(k, 0.0) + rng.gauss(0, p["sigma"]) for k in FEATURES}
        q = {"weights": w, "sigma": p["sigma"], "seed": p["seed"]}
        s = held_out(q, rounds)
        if s > best_s:
            best, best_s = q, s
    adopt = best_s > cur + 0.01
    nxt = dict(best if adopt else p)
    nxt["sigma"] = min(4.0, p["sigma"] * 1.25) if adopt else max(0.05, p["sigma"] * 0.8)
    nxt["weights"] = {k: round(v, 4) for k, v in nxt["weights"].items()}
    fixed.check_policy(nxt)
    return nxt, adopt, cur, best_s
