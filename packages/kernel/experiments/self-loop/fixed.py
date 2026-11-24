"""E400: the parts of the self-improvement loop that the loop may not change.

The loop (loop.py) imports this file and never writes it. It holds:
- the test procedure's scoring: the two scores, their tolerances and the
  keep rule of docs/experiments/README.md, as E322's analyze.py applies it;
- the evaluator that judges a ranking policy (kept changes per test-hour);
- the guards: which paths are protected (tests, the bench and its scoring,
  this file, CI, the experiments README), the one-line diff a candidate may
  be, the branch names the loop may write, and the policy shape it may store.

A round pins a hash of every protected file at its start (manifest) and
checks it before every verdict, so a changed scorer stops the round.
"""
import fnmatch, hashlib, json, math, os, re, subprocess

COST = ["md", "pdf", "office", "invoices", "synthetic-history", "generated", "multiformat"]
NORTH = ["north/cases", "north/multiformat", "north/tables"]
# The four history corpora need pinned commits that are not in the rewritten
# history (E265, E266, E322): left out of the full run, as in E322 and E347.
FULL = COST + NORTH
TOL_N = 0.005  # NORTH_TOLERANCE in the README
TOL_C = 0.02  # the README's cost spread; a round widens it to its base runs' spread
BUDGET_SHARE = 0.5  # a policy is judged on the first half of a pool's test time

PROTECTED = [
    "packages/kernel/tests/*", "packages/kernel/examples/bench.rs",
    "packages/kernel/examples/common/*", "packages/kernel/experiments/self-loop/fixed.py",
    "packages/kernel/experiments/self-loop/catalog.tsv", "packages/kernel/Cargo.toml",
    "packages/kernel/Cargo.lock", ".github/*", "docs/experiments/README.md",
]
BRANCH = re.compile(r"^exp/400-(self-loop|cand-[a-z0-9-]+)$")
POLICY_KEYS = {"weights", "sigma", "seed"}


def protected(path):
    return any(fnmatch.fnmatch(path, p) for p in PROTECTED)


def manifest(root):
    """sha256 of every protected file under the git checkout at root."""
    files = subprocess.run(["git", "ls-files"], cwd=root, capture_output=True, text=True, check=True).stdout.split()
    h = {}
    for f in sorted(files):
        if protected(f):
            h[f] = hashlib.sha256(open(os.path.join(root, f), "rb").read()).hexdigest()
    return h


def check_branch(name):
    if not BRANCH.match(name):
        raise SystemExit(f"guard: the loop may not write branch {name}")


def check_diff(root, base, catalog_row, new_value):
    """A candidate is exactly one line of one catalog file: the constant's
    line with only its value changed. Anything else is refused."""
    out = subprocess.run(["git", "diff", "--unified=0", base, "--"], cwd=root, capture_output=True, text=True, check=True).stdout
    files = re.findall(r"^\+\+\+ b/(.*)$", out, re.M)
    minus = [l[1:] for l in out.splitlines() if l.startswith("-") and not l.startswith("---")]
    plus = [l[1:] for l in out.splitlines() if l.startswith("+") and not l.startswith("+++")]
    want_file = "packages/kernel/" + catalog_row["file"]
    old = catalog_row["line"]
    new = old.replace(f"= {catalog_row['value']};", f"= {new_value};")
    ok = files == [want_file] and minus == [old] and plus == [new] and not protected(want_file)
    if not ok:
        raise SystemExit(f"guard: candidate diff is not the one allowed line ({files}, {minus}, {plus})")


def check_policy(p):
    if set(p) != POLICY_KEYS or not all(isinstance(v, (int, float)) for v in p["weights"].values()):
        raise SystemExit("guard: a policy is weights, sigma and seed, all numbers")


def lines(text):
    out = {}
    for l in text.splitlines():
        if l.startswith("{"):
            d = json.loads(l)
            out[d["corpus"]] = d
    return out


def scores(text):
    """(cost score, north score) of one bench run; either may be None."""
    d = lines(text)
    cost = d.get("_summary", {}).get("score")
    north = d.get("_north", {}).get("north_score")
    if north is not None:  # four decimals on _north; the corpora give it to their own
        vs = [d[c]["vs_dict"] for c in NORTH if c in d]
        north = math.exp(sum(map(math.log, vs)) / len(vs)) if len(vs) == len(NORTH) else north
    return cost, north


def cost_tol(base_costs):
    """The cost tolerance of a round: the README's 2%, or the furthest base
    run from the base median when that is wider (E322)."""
    m = sorted(base_costs)[len(base_costs) // 2]
    return max(TOL_C, max(abs(x / m - 1) for x in base_costs) + 0.0005)


def cls(xs, t):
    return -1 if all(x < -t for x in xs) else (1 if all(x > t for x in xs) else 0)


def verdict(dn_north_run, dn_full, dc_full, tol_c):
    """The keep rule for a candidate measured north first. dn_north_run: the
    north score's change from the north-only run; dn_full and dc_full: the
    north and cost changes of its two full runs (empty when not run). North
    worse or flat closes it (flat cannot be kept without a cost win, which
    this procedure does not look for); better needs two full runs with north
    still better and cost not worse on both beyond tol_c."""
    if dn_north_run > TOL_N:
        return "worse"
    if dn_north_run >= -TOL_N:
        return "hold"
    if len(dc_full) < 2:
        return "unfinished"
    sn, sc = cls(dn_full, TOL_N), cls(dc_full, tol_c)
    return "keep" if sn == -1 and sc <= 0 else ("worse" if sc == 1 else "hold")


def walk(order, outcomes, budget_s):
    """Spend test time down a ranked list until the budget runs out."""
    spent = kept = 0
    for c in order:
        o = outcomes[c]
        if spent + o["secs"] > budget_s:
            break
        spent += o["secs"]
        kept += o["verdict"] == "keep"
    return kept, spent


def kept_per_hour(order, outcomes):
    """The loop's own score: changes kept per test-hour on the first
    BUDGET_SHARE of the pool's total test time, and the area under the
    kept-against-hours curve over the whole pool (0 to 1, near 1 when every
    kept change comes first)."""
    total = sum(o["secs"] for o in outcomes.values())
    kept, spent = walk(order, outcomes, BUDGET_SHARE * total)
    n_keep = sum(o["verdict"] == "keep" for o in outcomes.values())
    area = t = 0.0
    k = 0
    for c in order:
        area += k * outcomes[c]["secs"]
        k += outcomes[c]["verdict"] == "keep"
        t += outcomes[c]["secs"]
    best = n_keep * total if n_keep else 1
    return {"kept": kept, "hours": spent / 3600, "per_hour": kept / (spent / 3600) if spent else 0.0,
            "auc": area / best if n_keep else 0.0, "keeps_in_pool": n_keep}
