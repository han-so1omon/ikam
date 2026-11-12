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
