# Test Fixtures (IKAM)

This directory contains JSON fixtures for IKAM provenance/export tests.

- `ikam_test_data.json`: Project, artifact, and fragment IDs used in tests.
- `ikam_provenance_snapshot.json`: A snapshot of provenance entities for deterministic checks.

Fixtures were snapshot-copied from the source monorepo during IKAM repo extraction.
Regeneration tooling is intentionally not included here until a package-local replacement exists.

Use deterministic mode in export tests via `?deterministic=1` for stable bytes.
