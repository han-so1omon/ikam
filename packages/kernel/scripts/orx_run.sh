#!/bin/sh
# The fixed OpenResearch run command for this branch's experiment
# (`sh packages/kernel/scripts/orx_run.sh` on every node); each branch
# changes only what it runs. Synthetic data only.
set -eu
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${TMPDIR:-/tmp}/ikam-orx-target}"
cargo build --release -q --features search-tantivy --example search_segments
cargo test --release -q --test textindex
"$CARGO_TARGET_DIR/release/examples/search_segments" 50000 0.1 10 100 1000
