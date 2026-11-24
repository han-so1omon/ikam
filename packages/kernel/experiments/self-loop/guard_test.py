"""E400: the loop's guards refuse what they should, in a scratch git repo.

    python3 -I experiments/self-loop/guard_test.py
"""
import os, subprocess, sys, tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import fixed  # noqa: E402

LINE = "const GAP_DIFF: usize = 1024;"
ROW = {"file": "src/template.rs", "line": LINE, "value": "1024"}


def git(root, *a):
    subprocess.run(["git", *a], cwd=root, check=True, capture_output=True)


def refused(f, *a):
    try:
        f(*a)
    except SystemExit:
        return True
    return False


def main():
    root = tempfile.mkdtemp()
    for p, text in [("packages/kernel/src/template.rs", f"{LINE}\nconst HOLE_COST: usize = 8;\n"),
                    ("packages/kernel/tests/t.rs", "assert!(true);\n"),
                    ("packages/kernel/examples/bench.rs", "const ARCHIVE_READS: f64 = 2.0;\n"),
                    (".github/workflows/ci.yml", "on: push\n")]:
        os.makedirs(os.path.dirname(os.path.join(root, p)), exist_ok=True)
        open(os.path.join(root, p), "w").write(text)
    git(root, "init", "-q")
    git(root, "add", ".")
    git(root, "-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qm", "base")
    base = subprocess.run(["git", "rev-parse", "HEAD"], cwd=root, capture_output=True, text=True).stdout.strip()
    pins = fixed.manifest(root)
    checks = []

    def edit(path, old, new):
        p = os.path.join(root, path)
        text = open(p).read()
        open(p, "w").write(text.replace(old, new))

    edit("packages/kernel/src/template.rs", "= 1024;", "= 256;")
    checks.append(("the one allowed line passes", not refused(fixed.check_diff, root, base, ROW, "256")))
    edit("packages/kernel/tests/t.rs", "true", "false")
    checks.append(("a test edited beside it is refused", refused(fixed.check_diff, root, base, ROW, "256")))
    checks.append(("the manifest sees the test change", fixed.manifest(root) != pins))
    git(root, "checkout", "-q", "--", "packages/kernel/tests/t.rs")
    edit("packages/kernel/examples/bench.rs", "2.0", "8.0")
    checks.append(("a scoring constant edited beside it is refused", refused(fixed.check_diff, root, base, ROW, "256")))
    git(root, "checkout", "-q", "--", ".")
    edit("packages/kernel/src/template.rs", "= 8;", "= 4;")
    checks.append(("another line of the same file is refused", refused(fixed.check_diff, root, base, ROW, "256")))
    git(root, "checkout", "-q", "--", ".")
    edit(".github/workflows/ci.yml", "push", "never")
    checks.append(("a CI change is seen by the manifest", fixed.manifest(root) != pins))
    checks.append(("main is not a branch the loop writes", refused(fixed.check_branch, "main")))
    checks.append(("a candidate branch is", not refused(fixed.check_branch, "exp/400-cand-gap-diff-256")))
    checks.append(("a policy with another key is refused",
                   refused(fixed.check_policy, {"weights": {}, "sigma": 1, "seed": 1, "keep_rule": "any"})))
    checks.append(("the catalog and the scorer are protected",
                   fixed.protected("packages/kernel/experiments/self-loop/fixed.py")
                   and fixed.protected("packages/kernel/experiments/self-loop/catalog.tsv")))
    checks.append(("a catalog row on a protected path is refused",
                   refused(fixed.check_diff, root, base, {"file": "../kernel/tests/t.rs", "line": "assert!(true);", "value": "true"}, "false")))
    rows = [dict(ROW, name="gap-diff", tries=["256", "4096"]), {"name": "hole-cost", "file": "src/template.rs",
            "line": "const HOLE_COST: usize = 8;", "value": "8", "tries": ["4", "16"]}]
    git(root, "checkout", "-q", "--", ".")
    git(root, "checkout", "-q", "-b", "p1")
    edit("packages/kernel/src/template.rs", "= 1024;", "= 256;")
    git(root, "-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qam", "p1")
    p1 = subprocess.run(["git", "rev-parse", "HEAD"], cwd=root, capture_output=True, text=True).stdout.strip()
    checks.append(("a parent of catalog lines is accepted", not refused(fixed.check_chain, root, base, p1, rows)))
    edit("packages/kernel/src/template.rs", "= 8;", "= 7;")
    git(root, "-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qam", "p2")
    p2 = subprocess.run(["git", "rev-parse", "HEAD"], cwd=root, capture_output=True, text=True).stdout.strip()
    checks.append(("a parent with a value the catalog does not try is refused", refused(fixed.check_chain, root, base, p2, rows)))
    git(root, "checkout", "-q", "p1")
    edit("packages/kernel/tests/t.rs", "true", "false")
    git(root, "-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qam", "p3")
    p3 = subprocess.run(["git", "rev-parse", "HEAD"], cwd=root, capture_output=True, text=True).stdout.strip()
    checks.append(("a parent that edits a test is refused", refused(fixed.check_chain, root, base, p3, rows)))
    checks.append(("a config cannot name a scoring step",
                   refused(fixed.check_config, {"steps": {"measure": "mine"}})
                   and refused(fixed.check_config, {"steps": {}, "tolerance": 0.1})))
    checks.append(("a config of the four step kinds passes",
                   not refused(fixed.check_config, {"steps": {"candidates": "catalog", "parents": "archive",
                                                              "novelty": "exact", "rank": "policy"}})))
    checks.append(("the net file, the config and the mechanism test are protected",
                   fixed.protected("packages/kernel/experiments/self-loop/self.net")
                   and fixed.protected("packages/kernel/experiments/self-loop/loop.json")
                   and fixed.protected("packages/kernel/experiments/self-loop/loop_test.py")))
    keep = {"name": "gap-diff", "base": "1024", "value": "256", "verdict": "keep"}
    checks.append(("a kept revert contradicts a keep",
                   fixed.contradicts(keep, {"name": "gap-diff", "base": "256", "value": "1024", "verdict": "keep"})))
    checks.append(("a held revert does not",
                   not fixed.contradicts(keep, {"name": "gap-diff", "base": "256", "value": "1024", "verdict": "hold"})))
    for name, ok in checks:
        print(f"{'ok  ' if ok else 'FAIL'} {name}")
    sys.exit(0 if all(ok for _, ok in checks) else 1)


main()
