"""Train the learned planner's trees (E425) from the bench's plan log.

    python3 -I train.py PLAN_LOG MODEL_OUT [--shuffle SEED] [--all] [--rate R]

PLAN_LOG is what `cargo run --release --example bench -- --json --plan-log
FILE` writes: one JSON object a planning choice, tagged with its corpus.
Each site's candidate gets one small decision tree (scikit-learn) that
predicts whether building it pays: building pays when skipping it would
lose more bytes than RATE times the milliseconds building it takes. RATE is
1 byte a millisecond, so nearly any byte lost outweighs the time: the north
score counts bytes only and holds within 0.5%. (At the cost score's own
price, a millisecond of CPU a month against 480 bytes stored a month, $0.04
a vCPU-hour against $0.023 a GB-month, every grouping in the plan log costs
more time than it saves bytes, and a model trained at that price drops
every group, north stores included; `--rate 480` trains it.) Each record is weighted by how much
the choice matters (that difference). Per site:

- group: one decision for the whole pass (every proposal, or none), since
  the proposals are near ties where any pays: what skipping them all loses
  is the best saving, against the time of every proposal;
- codec: each compressed encoding on its own (raw is always built): what
  skipping it loses is how much costlier the best of the others is.

Only records where the planner built the candidate count (a candidate
pruned exactly by its decode-cost floor is never asked about). Trees are
trained on the TRAIN corpora and scored on both splits; `--all` trains on
every corpus. `--shuffle SEED` permutes each tree's labels before training,
for the swap test (E051): a model fit to shuffled labels carries no
information from the features.

The model is written as plain text for `ikam_kernel::planner::Planner`:
`site candidate node feature threshold left right value`, feature -1 for a
leaf, whose value is the predicted chance that building pays.
"""

import json
import random
import sys

from sklearn.tree import DecisionTreeClassifier

TRAIN = {
    "md",
    "office",
    "synthetic-history",
    "generated",
    "transcripts",
    "north/cases",
    "north/tables",
}
RATE = 1.0  # bytes worth one millisecond of planning (see the docstring)
DEPTH = 3


def loss(rec, cand):
    """What skipping `cand` loses, and what building it costs, in bytes; or
    None when it was not built."""
    out, ms = rec["outcome"], rec["ms"]
    if out[cand] is None:
        return None
    built = [(c, o) for c, o in enumerate(out) if o is not None]
    if rec["site"] == "group":
        best = max([o for _, o in built] + [0.0])
        return best, RATE * sum(ms)
    best = min(o for _, o in built)
    rest = min(o for c, o in built if c != cand)
    return rest - best, RATE * ms[cand]


def gated(site):
    # Raw (codec 0) is always built: it is the plan every other must beat.
    return range(1, 5) if site == "codec" else range(5)


def tree_lines(site, cand, clf):
    t = clf.tree_
    lines = []
    for n in range(t.node_count):
        counts = t.value[n][0]
        classes = list(clf.classes_)
        p = counts[classes.index(1)] / counts.sum() if 1 in classes else 0.0
        if t.children_left[n] == -1:
            lines.append(f"{site} {cand} {n} -1 0 0 0 {p:.6f}")
        else:
            lines.append(
                f"{site} {cand} {n} {t.feature[n]} {t.threshold[n]:.6f} "
                f"{t.children_left[n]} {t.children_right[n]} {p:.6f}"
            )
    return lines


def chance(lines, f):
    nodes = {int(l.split()[2]): l.split() for l in lines}
    at = 0
    while True:
        _, _, _, feat, thr, left, right, value = nodes[at]
        if int(feat) < 0:
            return float(value)
        at = int(left) if f[int(feat)] <= float(thr) else int(right)


def evaluate(recs, model):
    """Per site: records, candidates skipped, build ms saved, and what the
    skips cost (group: bytes of saving lost; codec: cost units lost)."""
    out = {}
    for r in recs:
        s = out.setdefault(r["site"], {"records": 0, "skipped": 0, "ms_saved": 0.0,
                                       "ms_all": 0.0, "lost": 0.0, "wrong": 0})
        s["records"] += 1
        built = []
        for c, o in enumerate(r["outcome"]):
            s["ms_all"] += r["ms"][c]
            if o is None:
                continue
            lines = model.get((r["site"], c))
            keep = lines is None or chance(lines, r["f"]) >= 0.5
            if keep:
                built.append(c)
            else:
                s["skipped"] += 1
                s["ms_saved"] += r["ms"][c]
        vals = [(c, o) for c, o in enumerate(r["outcome"]) if o is not None]
        if r["site"] == "group":
            best = max([o for _, o in vals] + [0.0])
            got = max([o for c, o in vals if c in built] + [0.0])
            lost = best - got
        else:
            best = min(o for _, o in vals)
            got = min(o for c, o in vals if c in built or c == 0)
            lost = got - best
        s["lost"] += lost
        s["wrong"] += lost > 0
    return out


def main():
    args = sys.argv[1:]
    log, out_path = args[0], args[1]
    shuffle = int(args[args.index("--shuffle") + 1]) if "--shuffle" in args else None
    every = "--all" in args
    global RATE
    RATE = float(args[args.index("--rate") + 1]) if "--rate" in args else RATE
    recs = [json.loads(l) for l in open(log)]
    train = [r for r in recs if every or r["corpus"] in TRAIN]
    held = [r for r in recs if r["corpus"] not in TRAIN]
    lines = [
        "# ikam learned planner (E425), trained by experiments/learned-planner/train.py",
        f"# train corpora: {'all' if every else ','.join(sorted(TRAIN))}"
        + (f"; labels shuffled, seed {shuffle}" if shuffle is not None else ""),
    ]
    model = {}
    rng = random.Random(shuffle)
    for site in sorted({r["site"] for r in recs}):
        for cand in gated(site):
            xs, ys, ws = [], [], []
            for r in train:
                if r["site"] == site and (lc := loss(r, cand)) is not None:
                    xs.append(r["f"])
                    ys.append(int(lc[0] > lc[1]))
                    ws.append(max(abs(lc[0] - lc[1]), 1.0))
            if not xs:
                continue
            if shuffle is not None:
                rng.shuffle(ys)
            if len(set(ys)) < 2:
                # One class: a single leaf (a tree cannot split it).
                t = [f"{site} {cand} 0 -1 0 0 0 {float(ys[0]):.6f}"]
                model[(site, cand)] = t
                lines.extend(t)
                print(f"{site} {cand}: {len(xs)} records, {sum(ys)} pay, one leaf",
                      file=sys.stderr)
                continue
            clf = DecisionTreeClassifier(max_depth=DEPTH, min_samples_leaf=2,
                                         random_state=0)
            clf.fit(xs, ys, sample_weight=ws)
            t = tree_lines(site, cand, clf)
            model[(site, cand)] = t
            lines.extend(t)
            acc = sum(int(chance(t, x) >= 0.5) == y for x, y in zip(xs, ys)) / len(xs)
            print(f"{site} {cand}: {len(xs)} records, {sum(ys)} pay, "
                  f"train accuracy {acc:.3f}, nodes {len(t)}", file=sys.stderr)
    open(out_path, "w").write("\n".join(lines) + "\n")
    for name, part in [("train", train), ("held-out", held)]:
        for site, s in sorted(evaluate(part, model).items()):
            print(json.dumps({"split": name, "site": site, **{k: round(v, 1) if
                  isinstance(v, float) else v for k, v in s.items()}}))


if __name__ == "__main__":
    main()
