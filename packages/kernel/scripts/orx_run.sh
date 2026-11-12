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
cargo test --release -q --test prune_diff --test join
cargo build --release -q --example join_switch
for n in 20000 100000; do
  "$CARGO_TARGET_DIR/release/examples/join_switch" $n 1000 0.4 10 30 100 300 1000
done
