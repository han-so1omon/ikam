#!/bin/bash
# The bake-off measurements that need macOS (E122 to E152), for example-host.
#
# Safe by design: it runs as you, writes only under $ROOT, needs no sudo,
# loads no LaunchAgent, opens no port, and leaves /path/to/ikam,
# ikam-live and ikam-test alone (the repository is cloned, not changed).
# Delete $ROOT afterwards to undo everything.
#
#   bash mac-bakeoff.sh            # every step
#   bash mac-bakeoff.sh flush goal # only some steps
#
# Steps: preflight build flush crash apfs goal clone summary
# Results: $ROOT/results/*.jsonl and $ROOT/results/summary.txt.
set -euo pipefail

ROOT=${ROOT:-/path/to/ikam-bakeoff}
SRC=${SRC:-/path/to/ikam}         # cloned from, never changed
BRANCH=${BRANCH:-exp/152-held-index}
KILLS=${KILLS:-50}                       # crash runs per backend
OBJECTS=${OBJECTS:-"100000 1000000"}     # store sizes for the APFS step
RUNS=${RUNS:-3}                          # goal runs per store

REPO=$ROOT/repo
K=$REPO/packages/kernel
export CARGO_TARGET_DIR=$ROOT/target
X=$CARGO_TARGET_DIR/release
OUT=$ROOT/results
W=$ROOT/work
mkdir -p "$OUT" "$W"
log() { printf '%s %s\n' "$(date -u +%H:%M:%S)" "$*" | tee -a "$OUT/run.log"; }
STEPS="$*"   # a plain string: macOS bash 3.2 treats an empty array as unset
want() { [ -z "$STEPS" ] || [[ " $STEPS " == *" $1 "* ]]; }

if want preflight; then
  [ "$(uname)" = Darwin ] || { echo "run this on the Mac"; exit 1; }
  [ "$(id -u)" != 0 ] || { echo "run as yourself, not root"; exit 1; }
  free_gb=$(df -g "$ROOT" | awk 'NR==2 {print $4}')
  [ "$free_gb" -ge 20 ] || { echo "need 20 GB free under $ROOT, have ${free_gb} GB"; exit 1; }
  {
    echo "{\"step\":\"preflight\",\"host\":\"$(scutil --get LocalHostName)\",\"macos\":\"$(sw_vers -productVersion)\",\"model\":\"$(sysctl -n hw.model)\",\"cpus\":$(sysctl -n hw.ncpu),\"free_gb\":$free_gb,\"fs\":\"$(diskutil info "$(df "$ROOT" | awk 'NR==2 {print $1}')" | awk -F: '/Type \(Bundle\)/ {gsub(/ /,"",$2); print $2}')\"}"
  } > "$OUT/preflight.jsonl"
  log "preflight: $(cat "$OUT/preflight.jsonl")"
fi

if want build; then
  if [ ! -d "$REPO/.git" ]; then
    git clone --quiet "$SRC" "$REPO"
    git -C "$REPO" remote set-url origin "$(git -C "$SRC" remote get-url origin)"
  fi
  git -C "$REPO" fetch --quiet origin "$BRANCH"
  git -C "$REPO" checkout --quiet --detach FETCH_HEAD
  log "build: $BRANCH at $(git -C "$REPO" rev-parse --short HEAD)"
  (cd "$K" && cargo build --release --quiet --features stores \
     --example stores --example goal --bin ikam)
  git -C "$REPO" rev-parse HEAD > "$OUT/commit.txt"
fi

# 1. What a durable commit costs. On macOS Rust's sync_all is F_FULLFSYNC (a
#    drive cache flush); on Linux files made 66 flushes a commit, packs 2,
#    SQLite and LMDB about 1. sqlite is fullfsync on, sqlite-normal off.
if want flush; then
  log "flush: 1,000 single commits, 10 batches, 1,000 ref swaps per backend"
  "$X/examples/stores" db "$W/db" > "$OUT/flush.jsonl"
  rm -rf "$W/db"
fi

# 2. No acknowledged commit lost when a writer is killed (SIGKILL only; a
#    power cut is not tested).
if want crash; then
  : > "$OUT/crash.jsonl"
  for b in files packs sqlite lmdb; do
    log "crash: $b, $KILLS kills"
    "$X/examples/stores" crash "$W/crash" "$b" "$KILLS" >> "$OUT/crash.jsonl"
  done
  rm -rf "$W/crash"
fi

# 3. APFS: one file per object against packs, by store size: build time,
#    disk use against logical bytes, open and lookup times. The page cache
#    is not dropped (that needs sudo), so "cold" means a fresh process.
if want apfs; then
  : > "$OUT/apfs.jsonl"
  for n in $OBJECTS; do
    for b in files packs; do
      d=$W/apfs-$b-$n
      log "apfs: $b, $n objects"
      "$X/examples/stores" build "$b" "$d" "$n" >> "$OUT/apfs.jsonl"
      "$X/examples/stores" probe "$b" "$d" "$n" >> "$OUT/apfs.jsonl"
      echo "{\"step\":\"apfs_du\",\"backend\":\"$b\",\"n\":$n,\"disk_kb\":$(du -sk "$d" | cut -f1),\"files\":$(find "$d" -type f | wc -l | tr -d ' ')}" >> "$OUT/apfs.jsonl"
      rm -rf "$d"
    done
  done
fi

# 4. E133's two goals with the recommended setup (E152's), on one file per
#    object and on packs. Each run copies the cases store into a fresh store.
if want goal; then
  if [ ! -d "$W/cases.ikam" ]; then
    log "goal: ingesting tests/fixtures/cases (about 626 MB in, 229 MB stored)"
    "$X/ikam" --store "$W/cases.ikam" ingest "$REPO/tests/fixtures/cases" --ref c/cases > "$OUT/cases-ingest.txt"
  fi
  : > "$OUT/goal.jsonl"
  for st in files packs; do
    for r in $(seq "$RUNS"); do
      log "goal: $st, run $r"
      if [ "$st" = packs ]; then gs=pack; else gs=; fi
      GOAL_FS=$W/goalfs GOAL_STORE=$gs IKAM_CONFIG=recommended_held_index \
        "$X/examples/goal" "$W/cases.ikam" c/cases 2>/dev/null \
        | sed "s/^{/{\"store\":\"$st\",\"run\":$r,/" >> "$OUT/goal.jsonl"
    done
  done
  rm -rf "$W/goalfs"
fi

# 5. clonefile: a copy-on-write working copy of a store against a full copy.
if want clone; then
  [ -d "$W/cases.ikam" ] || { echo "run the goal step first"; exit 1; }
  : > "$OUT/clone.jsonl"
  for how in clone copy; do
    rm -rf "$W/cases-$how"
    t0=$(perl -MTime::HiRes=time -e 'print time')
    if [ $how = clone ]; then cp -cR "$W/cases.ikam" "$W/cases-$how"; else cp -R "$W/cases.ikam" "$W/cases-$how"; fi
    t1=$(perl -MTime::HiRes=time -e 'print time')
    free_after=$(df -k "$W" | awk 'NR==2 {print $4}')
    echo "{\"step\":\"clone\",\"how\":\"$how\",\"ms\":$(perl -e "printf '%.0f', ($t1-$t0)*1000"),\"free_kb_after\":$free_after}" >> "$OUT/clone.jsonl"
    rm -rf "$W/cases-$how"
  done
fi

if want summary; then
  python3 - "$OUT" <<'EOF' | tee "$OUT/summary.txt"
import json, sys, statistics, collections, os
out = sys.argv[1]
def rows(name):
    p = os.path.join(out, name)
    if not os.path.exists(p): return []
    r = []
    for line in open(p):
        try: r.append(json.loads(line))
        except ValueError: pass
    return r
for r in rows("preflight.jsonl"): print("machine:", r)
print("\nflush (per backend, as the harness printed it):")
for r in rows("flush.jsonl"): print(" ", r)
print("\ncrash:")
for r in rows("crash.jsonl"): print(" ", r)
print("\napfs:")
for r in rows("apfs.jsonl"): print(" ", r)
g = collections.defaultdict(list); fetched = {}
for r in rows("goal.jsonl"):
    if r.get("stage") and r.get("goal") not in (None, "-"):
        k = (r["store"], r["run"], r["goal"])
        g[k].append(r)
tot = collections.defaultdict(list)
for (st, run, goal), rs in g.items():
    tot[(st, goal)].append(sum(x.get("ms", 0) for x in rs))
    fetched[(st, goal)] = sum(x.get("fetched", 0) for x in rs)
print("\ngoal (median ms of the runs, bytes fetched):")
for k in sorted(tot): print(f"  {k[0]:5} {k[1]:3} {statistics.median(tot[k]):8.1f} ms  {fetched[k]:>9} B  runs {sorted(round(x,1) for x in tot[k])}")
print("\nclone:")
for r in rows("clone.jsonl"): print(" ", r)
EOF
  log "done; results in $OUT"
fi
