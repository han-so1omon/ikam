#!/usr/bin/env python3
"""Fail when text headed for this public repository carries private details.

Looks for general shapes only (home-folder paths, tailnet names, IPv4
addresses, key and token formats, private key blocks); it never holds anyone's
real details. Exceptions live in allowlist.txt next to this file.

  scan.py --range BASE..HEAD   added lines and commit messages in BASE..HEAD (CI)
  scan.py --staged             added lines in the index (pre-commit hook)
  scan.py --message FILE       a commit message (commit-msg hook)
  scan.py --all                every tracked text file as it is now
"""
import argparse
import fnmatch
import ipaddress
import pathlib
import re
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent

RULES = [
    ("home-folder path", re.compile(r"(?<![\w.])/(?:Users|home)/[A-Za-z0-9._-]+/|\b[A-Za-z]:\\Users\\[A-Za-z0-9._ -]+\\")),
    ("tailnet name", re.compile(r"\b[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.ts\.net\b")),
    ("IPv4 address", re.compile(r"(?<![\w.])(?:\d{1,3}\.){3}\d{1,3}(?![\w.])")),
    ("GitHub token", re.compile(r"\b(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{40,})")),
    ("Anthropic or OpenAI key", re.compile(r"\bsk-(?:ant-)?[A-Za-z0-9_-]{32,}")),
    ("AWS access key", re.compile(r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b")),
    ("Google API key", re.compile(r"\bAIza[0-9A-Za-z_-]{35}\b")),
    ("Slack token", re.compile(r"\bxox[abprs]-[A-Za-z0-9-]{10,}")),
    ("private key", re.compile(r"-----BEGIN (?:[A-Z]+ )?PRIVATE KEY-----")),
]

# Addresses that name no one's machine: loopback, unspecified, broadcast and
# the documentation ranges (RFC 5737).
SAFE_NETS = [ipaddress.ip_network(n) for n in
             ("127.0.0.0/8", "0.0.0.0/32", "255.255.255.255/32",
              "192.0.2.0/24", "198.51.100.0/24", "203.0.113.0/24")]


def load_allowlist():
    patterns, paths = [], []
    for line in (HERE / "allowlist.txt").read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        kind, _, value = line.partition(" ")
        if kind == "path":
            paths.append(value.strip())
        elif kind == "text":
            patterns.append(re.compile(value.strip()))
        else:
            sys.exit(f"allowlist.txt: unknown line {line!r}")
    return patterns, paths


ALLOWED, ALLOWED_PATHS = load_allowlist()


def is_safe_ip(text):
    try:
        ip = ipaddress.ip_address(text)
    except ValueError:
        return True  # not an address (an octet over 255): a version or a number
    return any(ip in n for n in SAFE_NETS)


def findings(line):
    for name, rule in RULES:
        for m in rule.finditer(line):
            hit = m.group(0)
            if name == "IPv4 address" and is_safe_ip(hit):
                continue
            span = line[max(0, m.start() - 40):m.end() + 40]
            if any(a.search(span) for a in ALLOWED):
                continue
            yield name, hit


def path_allowed(path):
    return any(fnmatch.fnmatch(path, g) for g in ALLOWED_PATHS)


def git(*args):
    return subprocess.run(["git", *args], check=True, capture_output=True,
                          text=True, errors="replace").stdout


def added_lines(diff):
    """(path, line number, text) for each added line of a unified diff."""
    path, n = None, 0
    for line in diff.splitlines():
        if line.startswith("+++ "):
            path = line[6:] if line.startswith("+++ b/") else None
        elif line.startswith("@@"):
            n = int(re.search(r"\+(\d+)", line).group(1))
        elif line.startswith("+") and path:
            yield path, n, line[1:]
            n += 1
        elif not line.startswith("-"):
            n += 1


def scan_diff(diff, out):
    for path, n, text in added_lines(diff):
        if not path_allowed(path):
            for name, hit in findings(text):
                out.append(f"{path}:{n}: {name}: {mask(hit)}")


def scan_message(label, text, out):
    for i, line in enumerate(text.splitlines(), 1):
        if line.startswith("#"):
            continue
        for name, hit in findings(line):
            out.append(f"{label}:{i}: {name}: {mask(hit)}")


def mask(hit):
    """Enough of a hit to find it, not enough to republish it in a CI log."""
    return hit[:3] + "..." if len(hit) <= 12 else hit[:4] + "..." + hit[-3:]


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--range")
    g.add_argument("--staged", action="store_true")
    g.add_argument("--message")
    g.add_argument("--all", action="store_true")
    a = ap.parse_args()
    out = []
    if a.range:
        scan_diff(git("diff", "--no-color", "--unified=0", "--no-ext-diff", a.range), out)
        for sha in git("rev-list", a.range).split():
            scan_message(f"commit {sha[:9]} message", git("log", "-1", "--format=%B", sha), out)
    elif a.staged:
        scan_diff(git("diff", "--cached", "--no-color", "--unified=0", "--no-ext-diff"), out)
    elif a.message:
        scan_message("commit message", pathlib.Path(a.message).read_text(errors="replace"), out)
    else:
        for path in git("ls-files", "-z").split("\0"):
            p = pathlib.Path(path)
            if not path or path_allowed(path) or not p.is_file():
                continue
            data = p.read_bytes()
            if b"\0" in data[:8192]:
                continue  # binary
            for i, line in enumerate(data.decode("utf-8", "replace").splitlines(), 1):
                for name, hit in findings(line):
                    out.append(f"{path}:{i}: {name}: {mask(hit)}")
    if out:
        print("Possible private details (this repository is public):", file=sys.stderr)
        print("\n".join(out), file=sys.stderr)
        print("Remove them, or add a narrow exception to .github/privacy/allowlist.txt.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
