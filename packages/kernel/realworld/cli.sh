#!/usr/bin/env bash
# Exercise the ikam CLI on a real corpus directory: ingest it into a fresh
# store, list it, search it, check it out again and compare with the source.
# Usage: realworld/cli.sh CORPUS_DIR [SEARCH_TEXT]
set -euo pipefail
src=$(cd "$1" && pwd)
text=${2:-the}
here=$(cd "$(dirname "$0")/.." && pwd)
ikam="$here/target/release/ikam"
[ -x "$ikam" ] || cargo build --release --manifest-path "$here/Cargo.toml" --bin ikam
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
store="$work/store"

"$ikam" --store "$store" ingest "$src" -m "$(basename "$src")"
"$ikam" --store "$store" ls main | head -5
echo "grep '$text': $("$ikam" --store "$store" grep "$text" | grep -vc '^  ') files"
"$ikam" --store "$store" stats | head -3
"$ikam" --store "$store" checkout main "$work/out"
diff -r "$src" "$work/out" && echo "checkout identical to source"
