"""E305: does a fusion setting tuned on one dataset carry to another?

    python3 carry.py NAME=SWEEP.jsonl [NAME=SWEEP.jsonl ...]

Each SWEEP.jsonl is `fusion_tune`'s output for one dataset. For every
pair (tuned on A's dev, scored on B's test), and for each way (convex,
rrf), prints B's test nDCG@10 with A's best setting, beside B's own best
on dev and the defaults (alpha 0.5, k 60). One JSON line each.
"""
import json, sys

sweeps = {}
for arg in sys.argv[1:]:
    name, path = arg.split("=", 1)
    for l in open(path):
        r = json.loads(l)
        if "dev_ndcg_at_10" in r:
            key = ("convex", round(r["alpha"], 1)) if r["way"] == "convex" else ("rrf", r["k"])
            sweeps.setdefault(name, {}).setdefault(r["task"], {})[key] = (r["dev_ndcg_at_10"], r["test_ndcg_at_10"])

sets = [(n, t, v) for n, tasks in sweeps.items() for t, v in tasks.items()]
for way, default in [("convex", 0.5), ("rrf", 60)]:
    best = {}
    for n, t, v in sets:
        opts = {k: x for k, x in v.items() if k[0] == way}
        best[(n, t)] = max(opts, key=lambda k: opts[k][0])
    for nb, tb, vb in sets:
        for na, ta, _ in sets:
            k = best[(na, ta)]
            print(json.dumps({
                "way": way, "tuned_on": f"{na}/{ta}", "scored_on": f"{nb}/{tb}",
                "setting": k[1], "test_ndcg_at_10": vb[k][1],
                "own_dev_best": best[(nb, tb)][1], "own_dev_best_test": vb[best[(nb, tb)]][1],
                "default_test": vb[(way, default)][1],
            }))
