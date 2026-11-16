"""E322: score pilot runs against full runs (see drive.py).

Full verdict per variant, from its two full runs against the pooled base
runs: the keep rule (docs/experiments/README.md), with the cost score's
tolerance given (2% as in the README, and this machine's measured base
spread) and the north score's 0.5%. A pilot's change is its corpus line
(cost corpora: vs_baseline_cost; north corpora: vs_dict) against the base
pilots' median. Reported: each corpus's noise, how often a pilot's sign
matches the sign of the score it belongs to (against always guessing
"flat"), and pilot gates that stop a candidate when the pilot is worse
than a threshold.

    python3 -I experiments/pilot-runs/analyze.py [--exclude base/full2 ...]
"""
import json, math, os, statistics as st, sys

K = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
R = os.environ.get("PILOT_RUNS", os.path.join(K, "target", "pilot-runs"))
COST = ["md", "pdf", "office", "invoices", "synthetic-history", "generated", "multiformat"]
NORTH = ["north/cases", "north/multiformat", "north/tables"]
ALL = COST + NORTH
TOL_N = 0.005
EXCLUDE = {a for i, a in enumerate(sys.argv) if i > 0 and sys.argv[i - 1] == "--exclude"}


def lines(p):
    out = {}
    if os.path.exists(p):
        for l in open(p):
            if l.startswith("{"):
                d = json.loads(l)
                out[d["corpus"]] = d
    return out


def metric(d, c):
    return d[c]["vs_dict"] if c in NORTH else d[c]["vs_baseline_cost"]


def load(name):
    d = os.path.join(R, name)
    meta = json.load(open(os.path.join(d, "meta.json")))
    fulls = [lines(f"{d}/full{i}.jsonl") for i in range(len(meta["full"])) if f"{name}/full{i}" not in EXCLUDE]
    pil = {c: [lines(f"{d}/pilot-{c.replace('/', '_')}-{i}.jsonl") for i in range(len(meta["pilot"][c]))] for c in ALL}
    return meta, fulls, pil


def sign(x, t):
    return 0 if abs(x) <= t else (1 if x > 0 else -1)


def cls(xs, t):
    return -1 if all(x < -t for x in xs) else (1 if all(x > t for x in xs) else 0)


bases = [load(n) for n in sorted(os.listdir(R)) if n.startswith("base")]
bfull = [f for b in bases for f in b[1]]
bpil = {c: [r for b in bases for r in b[2][c]] for c in ALL}
costs = [f["_summary"]["score"] for f in bfull]
b_cost = st.median(costs)
b_north = st.median(f["_north"]["north_score"] for f in bfull)
b_full = st.median(w for b in bases for w, _ in b[0]["full"])
b_wall = {c: st.median(w for b in bases for w, _ in b[0]["pilot"][c]) for c in ALL}
bp = {c: st.median(metric(r, c) for r in bpil[c]) for c in ALL}
noise = {c: max(abs(x / bp[c] - 1) for x in [metric(r, c) for r in bpil[c]] + [metric(f, c) for f in bfull]) for c in ALL}
tol = {c: TOL_N if c in NORTH else max(0.02, noise[c]) for c in ALL}
spread = max(abs(x / b_cost - 1) for x in costs)
print(f"base: {len(bfull)} full runs, cost {costs} (median {b_cost:.4f}, furthest {100*spread:.1f}% from it), north {b_north:.4f}; a full run {b_full:.0f} s")
for c in ALL:
    print(f"  {c:18} alone {b_wall[c]:5.1f} s, noise {100*noise[c]:4.1f}%")

V = []
for name in sorted(os.listdir(R)):
    if name.startswith("base"):
        continue
    meta, fulls, pil = load(name)
    V.append(dict(name=name,
                  dc=[f["_summary"]["score"] / b_cost - 1 for f in fulls],
                  dn=[f["_north"]["north_score"] / b_north - 1 for f in fulls],
                  pv={c: metric(pil[c][0], c) / bp[c] - 1 for c in ALL}))
TCS = [0.02, round(spread + 0.0005, 3)]
for v in V:
    v["sn"] = cls(v["dn"], TOL_N)
    for tc in TCS:
        sc = cls(v["dc"], tc)
        v[("sc", tc)] = sc
        v[("verdict", tc)] = "keep" if ((sc == -1 and v["sn"] <= 0) or (v["sn"] == -1 and sc <= 0)) else ("worse" if 1 in (sc, v["sn"]) else "hold")
n = len(V)
print()
print("variant              cost run 1, run 2   north   verdict (cost tol " + ", ".join(f"{100*t:.1f}%" for t in TCS) + ")  | pilots, % change")
for v in V:
    print(f"{v['name']:20} {100*v['dc'][0]:+6.1f}% {100*v['dc'][1]:+6.1f}%  {100*v['dn'][0]:+6.2f}%  "
          + " ".join(f"{v[('verdict', t)]:5}" for t in TCS) + "  | "
          + " ".join(f"{c.split('/')[-1][:5]} {100*v['pv'][c]:+.1f}" for c in ALL))
for tc in TCS:
    print(f"cost tol {100*tc:.1f}%: keep {[v['name'] for v in V if v[('verdict', tc)] == 'keep']}, "
          f"worse {sum(v[('verdict', tc)] == 'worse' for v in V)}, hold {sum(v[('verdict', tc)] == 'hold' for v in V)}")
print("\nsign of one corpus alone against the sign of its score (better, flat, worse):")
for c in ALL:
    if c in NORTH:
        print(f"  {c:18} north: {sum(sign(v['pv'][c], tol[c]) == v['sn'] for v in V)}/{n} (always flat: {sum(v['sn'] == 0 for v in V)}/{n})")
    else:
        print(f"  {c:18} cost:  " + ", ".join(
            f"{sum(sign(v['pv'][c], tol[c]) == v[('sc', t)] for v in V)}/{n} at tol {100*t:.1f}% (always flat {sum(v[('sc', t)] == 0 for v in V)}/{n})" for t in TCS))


def gm(v, cs):
    return math.exp(sum(math.log(1 + v["pv"][c]) for c in cs) / len(cs)) - 1


print("\ngates: stop when the pilot (geometric mean over its corpora) is worse than a threshold;"
      " time against two full runs per candidate, as the keep rule needs")
tc = TCS[-1]
keeps = sum(v[("verdict", tc)] == "keep" for v in V)
worse = sum(v[("verdict", tc)] == "worse" for v in V)
gates = []
for cs in [[c] for c in NORTH] + [["north/multiformat", "north/tables"], NORTH] + [[c] for c in COST]:
    pt = sum(b_wall[c] for c in cs)
    for th in ([0.005, 0.01, 0.02] if cs[0] in NORTH else [tol[cs[0]]]):
        stop = [v for v in V if gm(v, cs) > th]
        saved = len(stop) * 2 * b_full - n * pt
        g = dict(pilot="+".join(cs), seconds=round(pt, 1), threshold=th, stops=len(stop),
                 worse_caught=sum(v[("verdict", tc)] == "worse" for v in stop),
                 keeps_stopped=sum(v[("verdict", tc)] == "keep" for v in stop),
                 time_saved=round(saved / (n * 2 * b_full), 3), stopped=[v["name"] for v in stop])
        gates.append(g)
        print(f"  {g['pilot']:34} {pt:5.1f} s, above {100*th:4.1f}%: stops {len(stop):2}, worse caught {g['worse_caught']}/{worse},"
              f" keeps stopped {g['keeps_stopped']}/{keeps}, bench time {100*g['time_saved']:+.0f}%")
json.dump(dict(base_cost=b_cost, base_north=b_north, full_s=b_full, pilot_s=b_wall, noise=noise, variants=V and [
    {k if isinstance(k, str) else f"{k[0]}@{k[1]}": val for k, val in v.items()} for v in V], gates=gates),
    open(os.path.join(R, "summary.json"), "w"), indent=1)
