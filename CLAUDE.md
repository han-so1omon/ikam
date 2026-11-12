# CLAUDE.md

See AGENTS.md for the engineering rules.

## Public repository: no private details

This repository is public. Never commit:

- machine hostnames or tailnet names
- local folder paths
- user names
- service or LaunchAgent names
- ports of private deployments
- IP addresses
- credentials, keys or tokens
- personal data
- data or numbers from private projects

Use placeholders such as `example-host` and `/path/to`, or environment variables, instead.

Before every commit and push, scan the diff (including commit messages) for these and remove them.

### The privacy check

`.github/privacy/scan.py` looks for home-folder paths, tailnet names, IPv4 addresses and common key or token shapes. The `privacy` workflow runs it on every push and pull request, over the added lines and commit messages, and over the whole tree.

Run the same check locally before each commit, commit message and push:

```
git config core.hooksPath .github/privacy/hooks
```

Scan by hand with `python3 .github/privacy/scan.py --all` (whole tree) or `--range main..HEAD`. A false positive gets a narrow line in `.github/privacy/allowlist.txt`, with a comment saying why. Never put a real private value there.
