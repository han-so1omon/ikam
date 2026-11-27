"""Novelty: skip a candidate before it is tested (E401, after ShinkaEvolve).

- `exact`: the same set of changes already measured on the same base.
- `exact+hints`: also, by the graph's hints at this base: adding a constant
  that does not move the north score (the child would measure as its
  parent), adding a change that alone made north worse, and a set of
  changes equal to a measured one once its non-moving constants are
  dropped. The second can skip a change that only helps on another one; it
  is a choice of this mode, not of the keep rule.
- `embed`: also a candidate whose description is within `threshold` cosine
  of one already measured, by IKAM's bge-small embedder (E387, the `embed`
  feature and a model folder in cfg["novelty"]["model"]). Needs the model's
  weights, which this cloud workspace cannot download; it is not used here.
"""
import json, subprocess


def key(changes):
    return tuple(sorted((c["name"], str(c["value"])) for c in changes))


def describe(changes):
    return "; ".join(f"set {c['name']} from {c['base']} to {c['value']}" for c in sorted(changes, key=lambda c: c["name"]))


def repeat(cfg, graph, base_commit, changes, added=None):
    """None, or why the candidate is a repeat."""
    kind = cfg["steps"]["novelty"]
    k = key(changes)
    done = {key(f["changes"]): f["token"] for f in graph.values()
            if f.get("kind") == "measured" and f["base_commit"] == base_commit}
    if k in done:
        return "measured as " + done[k]
    if kind in ("exact+hints", "embed"):
        hints = [h for h in graph.values() if h.get("base_commit") == base_commit]
        flat = {h["constant"] for h in hints if h.get("kind") == "zero-move"}
        worse = {(h["constant"], str(h["value"])) for h in hints if h.get("kind") == "worse-alone"}
        if added and added["name"] in flat:
            return f"{added['name']} does not move north at this base (hint)"
        if added and (added["name"], str(added["value"])) in worse:
            return f"{added['name']} = {added['value']} alone makes north worse at this base (hint)"
        eff = key([c for c in changes if c["name"] not in flat])
        if eff != k and eff in done:
            return "same as " + done[eff] + " without its constants that do not move north (hint)"
    if kind == "embed":
        return near(cfg, graph, base_commit, changes)
    return None


def near(cfg, graph, base_commit, changes):
    c = cfg["novelty"]
    done = [f for f in graph.values() if f.get("kind") == "measured" and f["base_commit"] == base_commit]
    texts = [describe(changes)] + [describe(f["changes"]) for f in done]
    r = subprocess.run(c["embed_cmd"] + [c["model"]], input="\n".join(texts), capture_output=True, text=True, check=True)
    vs = [json.loads(l) for l in r.stdout.splitlines()]
    q = vs[0]
    for f, v in zip(done, vs[1:]):
        if sum(a * b for a, b in zip(q, v)) >= c["threshold"]:
            return "near " + f["token"]
    return None
