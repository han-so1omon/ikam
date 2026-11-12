#!/bin/sh
# The fixed OpenResearch run command for this branch's experiment
# (`sh packages/kernel/scripts/orx_run.sh` on every node); each branch
# changes only what it runs. Synthetic data only.
set -eu
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${TMPDIR:-/tmp}/ikam-orx-target}"
# Runs unpack sources with old modification times; touch them so cargo
# rebuilds this crate instead of reusing another branch's build.
find . -name '*.rs' -exec touch {} +
cargo test --release -q --test colblock
cargo build --release -q --example fast_fields
"$CARGO_TARGET_DIR/release/examples/fast_fields" 100000 1000000
