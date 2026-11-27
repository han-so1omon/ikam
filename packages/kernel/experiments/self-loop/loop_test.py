"""E401: the loop's own mechanisms on a scratch store, with no benchmark.

Checks what a round only shows when it happens: a kept candidate that a
later kept one reverses is reopened (a new linked fact, never an edit) and
only once; `ikam net` never fires the owner's merge and refuses a firing
replayed with other output; the findings graph is committed to IKAM and
read back equal, merges by union and refuses a conflict; and the novelty
step skips exact repeats and what the hints rule out.

    python3 -I loop_test.py      # needs target/release/ikam
"""
import os, shutil, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import findings, loop  # noqa: E402
from steps import novelty  # noqa: E402


def refused(f, *a):
    try:
        f(*a)
    except SystemExit:
        return True
    return False


def candidate(store, tok, name, base, value, dn, verdict):
    loop.fire(store, "propose", tok, {"round": "t", "name": name, "base": base, "value": value})
    loop.fire(store, "start", tok)
    loop.fire(store, "finish", tok, {"round": "t", "name": name, "base": base, "value": value,
                                     "dn": dn, "secs": 1.0, "verdict": verdict})
    loop.fire(store, "keep" if verdict == "keep" else "close", tok)


def main():
    tmp = tempfile.mkdtemp(prefix="loop-test-")
    store = os.path.join(tmp, "s.ikam")
    checks = []
    try:
        loop.fire(store, "propose", "t-base", {"round": "t", "name": "base", "commit": "c0"})
        candidate(store, "t-a", "gap-diff", "1024", "256", -0.01, "keep")
        candidate(store, "t-b", "gap-diff", "256", "1024", -0.01, "keep")
        candidate(store, "t-c", "hole-cost", "8", "4", 0.0, "close")
        candidate(store, "t-d", "hole-cost", "8", "16", 0.0, "close")
        loop.reopen(store)
        h = loop.history(store)
        re = sorted(f["token"] for f in h if f["transition"] == "reopen")
        checks.append(("two keeps that reverse each other are both reopened", re == ["t-a", "t-b"]))
        loop.reopen(store)
        checks.append(("a second reopen pass fires nothing", len(loop.history(store)) == len(h)))
        g = loop.graph(store)
        con = [v for v in g.values() if v.get("kind") == "contradiction"]
        checks.append(("each reopen is a contradiction fact linked to its finding",
                       len(con) == 2 and all(v["by"] in g for v in con)))
        kept = [v for v in g.values() if v.get("kind") == "measured" and v["verdict"] == "keep"]
        checks.append(("the reopened facts are unchanged", len(kept) == 2))
        checks.append(("the committed graph reads back equal",
                       findings.checkout(loop.IKAM, store, os.path.join(tmp, "read")) == g))
        m = loop.net(store, "marking").stdout
        checks.append(("the reopened tokens left waiting", '"waiting":0' in m and '"reopened":2' in m))
        checks.append(("ikam net never fires the merge", loop.sh([loop.IKAM, "--store", store, "net", loop.NET,
                       "--ref", loop.CFG["ref"], "fire", "merge", "t-a"], loop.K, check=False).returncode != 0))
        checks.append(("a firing replayed with other output is refused",
                       refused(loop.fire, store, "propose", "t-a", {"round": "t", "name": "other"})))
        checks.append(("a firing replayed with the same output changes nothing",
                       not refused(loop.fire, store, "propose", "t-a",
                                   {"round": "t", "name": "gap-diff", "base": "1024", "value": "256"})))
        checks.append(("the net check passes", "replay equal" in loop.net(store, "check").stdout))
        checks.append(("a graph merges with itself", findings.merge(g, g) == g))
        other = {k: dict(v, secs=2.0) if v.get("kind") == "measured" else v for k, v in g.items()}
        checks.append(("a graph with other content under the same name is refused",
                       refused(findings.merge, g, other)))
        extra = {findings.fid("fact", {"x": 1}): {"x": 1}}
        checks.append(("two graphs merge by union", len(findings.merge(g, extra)) == len(g) + 1))
        cfg = dict(loop.CFG, steps=dict(loop.CFG["steps"], novelty="exact+hints"))
        gd = lambda v: {"name": "gap-diff", "base": "1024", "value": v}
        hc = lambda v: {"name": "hole-cost", "base": "8", "value": v}
        checks.append(("an exact repeat is skipped",
                       (novelty.repeat(cfg, g, "c0", [gd("256")], gd("256")) or "").startswith("measured as")))
        checks.append(("the same change at another base is not",
                       novelty.repeat(cfg, g, "c1", [gd("256")], gd("256")) is None))
        checks.append(("adding a constant that does not move north is skipped",
                       "hint" in (novelty.repeat(cfg, g, "c0", [gd("256"), hc("32")], hc("32")) or "")))
        checks.append(("a set equal to a measured one without such constants is skipped",
                       "same as" in (novelty.repeat(cfg, g, "c0", [gd("256"), hc("32")], gd("256")) or "")))
        checks.append(("exact mode keeps what only the hints rule out",
                       novelty.repeat(dict(cfg, steps=dict(cfg["steps"], novelty="exact")), g, "c0",
                                      [gd("256"), hc("32")], hc("32")) is None))
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    for name, ok in checks:
        print("ok  " if ok else "FAIL", name)
    sys.exit(0 if all(ok for _, ok in checks) else 1)


if __name__ == "__main__":
    main()
