"""Parents: what a candidate is built on.

- `main`: the round's base commit only (E400).
- `archive` (E401, stepping stones, after the Darwin Goedel Machine): every
  measured candidate that passed its gates is a possible parent, and the
  next one is drawn with weight sigmoid(score) / (1 + children), so a
  weaker variant with few children can be picked over the best one. The
  score is the north improvement in units of the north tolerance.
"""
import math

import fixed


def weight(f, children):
    s = 1 / (1 + math.exp(-(-(f["dn"] or 0.0) / fixed.TOL_N)))
    return s / (1 + children)


def archive(graph, base_commit):
    out = [f for f in graph.values() if f.get("kind") == "measured" and f["base_commit"] == base_commit
           and f["verdict"] != "gate-failed" and f["dn"] is not None]
    return sorted(out, key=lambda f: f["token"])


def children(graph, token):
    return sum(1 for f in graph.values() if f.get("kind") in ("measured", "intent") and f.get("parent") == token)


def pick(cfg, graph, base_commit, rng, exclude=()):
    """The next parent: None for the base commit, or a fact."""
    if cfg["steps"]["parents"] == "main":
        return None
    pool = [f for f in archive(graph, base_commit) if f["token"] not in exclude]
    root = {"token": None, "dn": 0.0}
    n_root = sum(1 for f in graph.values() if f.get("kind") == "measured" and f["base_commit"] == base_commit
                 and not f.get("parent"))
    cands = [(f, weight(f, children(graph, f["token"]))) for f in pool]
    if None not in exclude:
        cands.insert(0, (root, weight(root, n_root)))
    if not cands:
        raise StopIteration
    total = sum(w for _, w in cands)
    x = rng.random() * total
    for f, w in cands:
        x -= w
        if x <= 0:
            return None if f["token"] is None else f
    return None
