#!/bin/bash
# E440: demo projects for the explorer of `ikam host`, one for each way ikam
# is used, served together on 127.0.0.1:$PORT. The explorer's page lists
# them, each with its own page of links; /p/NAME/ opens a project.
#
#   ROOT=/path/to/folder bash scripts/explorer-demo.sh
#
# Safe by design: runs as you, needs no sudo, writes only under $ROOT (and
# the cargo target dir), listens on 127.0.0.1 only. A project already built
# in $ROOT/stores is kept; delete $ROOT to build all again.
#
# Settings (environment):
#   PORT      127.0.0.1 port (default 18760); must be free, or this script's
#             own host from an earlier run (it is stopped first)
#   EXPLORER  host names that answer with the explorer, space separated
#             (default "localhost"), such as the name a proxy passes on
#   VIEWER    an `ikam passwd` line (USER:SALT:HASH) made owner of every demo
#             project; without it a viewer password is made and kept in
#             $ROOT/viewer.pw (mode 600). Passwords are never printed.
#   EXTRA     more project names to make with a stub README, for pages that
#             only hold links (space separated)
#   LINKS     a file of more config lines (title, about, link) added last
#   ENWIK     a MediaWiki dump (such as enwik8) for the wiki demo; PAGES of
#             its pages (default 3000) are stored one file each and rebuilt
#   SERVE=1   leave the host running (nohup; pid in $ROOT/host.pid)
#
# Prints one line per check ("name: got X, want Y") and a summary.
set -uo pipefail

K=$(cd "$(dirname "$0")/.." && pwd)    # packages/kernel of this checkout
REPO=$(cd "$K/../.." && pwd)
PY="python3 -I $K/scripts/explorer-demo.py"
ROOT=${ROOT:?set ROOT to a folder for the demo}
PORT=${PORT:-18760}
EXPLORER=${EXPLORER:-localhost}
GH=https://github.com/han-so1omon/ikam
case "$ROOT" in *[[:space:]]*) echo "preflight: ROOT may not hold spaces"; exit 1 ;; esac
mkdir -p "$ROOT/src" "$ROOT/stores"
ROOT=$(cd "$ROOT" && pwd)
umask 077

if [ -f "$ROOT/host.pid" ]; then
  old=$(cat "$ROOT/host.pid")
  if ps -p "$old" -o command= 2>/dev/null | grep -q "host --config $ROOT/host.conf"; then
    echo "== stopping this demo's host from before (pid $old)"
    kill "$old"; for _ in $(seq 50); do kill -0 "$old" 2>/dev/null || break; sleep 0.2; done
  fi
  rm -f "$ROOT/host.pid"
fi
for p in "$PORT" $((PORT + 1)); do
  if command -v lsof >/dev/null && lsof -iTCP:"$p" -sTCP:LISTEN >/dev/null 2>&1; then
    echo "preflight: port $p is in use (the host takes PORT, the db demo PORT+1); set PORT"; exit 1
  fi
done

echo "== build"
(cd "$K" && cargo build --release -q --bin ikam --example net_night_plan --example loop_drive) \
  || { echo "build: failed"; exit 1; }
T="${CARGO_TARGET_DIR:-$K/target}/release"
B="$T/ikam"
[ -x "$B" ] || { echo "build: no ikam binary in $T"; exit 1; }

pw() { head -c 18 /dev/urandom | base64 | tr -d '/+=' ; }
mk() { printf '%s\n' "$2" | "$B" passwd "$1" 2>/dev/null | tail -1 | sed 's/^IKAM_AUTH=//'; }
[ -s "$ROOT/admin.pw" ] || pw > "$ROOT/admin.pw"
ADMIN=$(mk demo-admin "$(cat "$ROOT/admin.pw")")
if [ -z "${VIEWER:-}" ]; then
  [ -s "$ROOT/viewer.pw" ] || pw > "$ROOT/viewer.pw"
  VIEWER=$(mk viewer "$(cat "$ROOT/viewer.pw")")
  echo "viewer: user viewer, password in $ROOT/viewer.pw"
fi
printf 'machine 127.0.0.1 login demo-admin password %s\n' "$(cat "$ROOT/admin.pw")" > "$ROOT/.netrc"

S="$ROOT/src"
fresh() { [ ! -e "$ROOT/stores/$1" ]; }
put() { "$B" --store "$ROOT/stores/$1" commit "$S/$1" -m "$2" >/dev/null 2>>"$ROOT/commit.log" || echo "commit $1: failed (see commit.log)"; }
readme() { mkdir -p "$S/$1"; cat > "$S/$1/README.md"; }

echo "== demo projects"
if fresh files; then
  readme files <<'MD'
# Files

ikam as a filesystem: the same tree a mounted folder (WebDAV) and this page
edit. Open a file, edit it, upload, rename or delete; every change is a
commit (see *changes*). `data/cities-copy.csv` is a copy: the store keeps
its bytes once (see *store*).
MD
  mkdir -p "$S/files/data" "$S/files/notes"
  printf 'city,country,people\nLisbon,Portugal,545000\nOslo,Norway,709000\nKyoto,Japan,1460000\n' > "$S/files/data/cities.csv"
  printf '[{"city":"Lisbon","country":"Portugal"},{"city":"Oslo","country":"Norway"},{"city":"Kyoto","country":"Japan"}]\n' > "$S/files/data/cities.json"
  printf '# Notes\n\nFirst version.\n' > "$S/files/notes/today.md"
  cp "$REPO/README.md" "$S/files/ikam-readme.md"
  put files "files: first version"
  printf 'Second version: one more line.\n' >> "$S/files/notes/today.md"
  printf 'Porto,Portugal,232000\n' >> "$S/files/data/cities.csv"
  put files "files: edit a note and add a row"
  cp "$S/files/data/cities.csv" "$S/files/data/cities-copy.csv"
  put files "files: a copy stores nothing new"
fi
if fresh search; then
  readme search <<'MD'
# Search

ikam's own documents (plans, research notes, the experiment log). Search
finds text in every file; Ask gives the passages that answer a question
best. The project's page has a few to try.
MD
  mkdir -p "$S/search/docs" && cp -R "$REPO/docs/." "$S/search/docs/"
  put search "search: ikam's documents"
fi
if fresh lab; then
  readme lab <<'MD'
# Lab

The source side of a scoped share. `pub/` is shared with the project
*partner* (read only); `priv/` is not. Partner reads `pub/` through the
share and gets 404 for anything else, as if it did not exist.
MD
  mkdir -p "$S/lab/pub" "$S/lab/priv"
  printf '# Shared notes\n\nMeasurements anyone with the share may read.\n' > "$S/lab/pub/notes.md"
  printf 'sample,value\na,1.5\nb,2.25\n' > "$S/lab/pub/data.csv"
  printf '# Plan\n\nKept inside lab.\n' > "$S/lab/priv/plan.md"
  put lab "lab: shared and private files"
fi
if fresh partner; then
  readme partner <<'MD'
# Partner

A separate project: its own store, users and files. It sees lab's `pub/`
only through the share `pub`; the links on its page read through it.
MD
  put partner "partner: its own files"
fi
if fresh nets; then
  readme nets <<'MD'
# Petri nets

ikam runs work as Petri nets: places hold tokens, transitions fire, and each
firing is stored with its inputs and outputs. `night-plan.txt` plans a night
of experiments by simulating the experiment-loop net on the net's own
interpreter: serial against pipelined, with one or two cpus and owner
decisions that wait (`examples/net_night_plan.rs`).
MD
  "$T/examples/net_night_plan" > "$S/nets/night-plan.txt" 2>&1
  put nets "nets: a night planned on the loop net"
fi
if fresh loop; then
  readme loop <<'MD'
# Experiment loop

The loop that runs ikam's experiments, driven by its net one firing at a
time: `plan` queues experiments, `next` asks the net what to do, `start`
and `finish` fire, and `check` replays every firing from the store.
`night.txt` is such a run on a scratch store; `net.txt` is the net.
MD
  L="$ROOT/loop-store"; rm -rf "$L"
  {
    for a in "plan E386 E387 E388" next "start E386" next "finish E386" "keep E386" next "start E387" next check; do
      echo "\$ loop_drive STORE night $a"
      "$T/examples/loop_drive" "$L" night $a 2>&1 | grep -v '^ *[0-9]*: \|^stack\|^note:' | head -12
    done
  } > "$S/loop/night.txt"
  sed -n '/^const NIGHT: &str = "\\$/,/^";$/p' "$K/examples/loop_drive.rs" | sed '1d;$d' > "$S/loop/net.txt"
  put loop "loop: a night driven by its net"
fi
if fresh mcp; then
  readme mcp <<'MD'
# Agents (MCP)

`ikam mcp` serves a store as tools for an AI agent (list, read, search,
rank, write, edit, commit with a message, diff, history, drafts).
`session.md` is a session against the *search* project's documents.
MD
  $PY mcp "$B" "$ROOT/stores/search" "$S/mcp/session.md"
  put mcp "mcp: a recorded session"
fi
if fresh db; then
  readme db <<'MD'
# Database

`ikam db serve`: records for programs, every write committed before it is
answered, every earlier version readable. `session.md` appends, reads,
changes a record and reads it at the first commit again.
MD
  rm -rf "$ROOT/db-store"
  DBPORT=$((PORT + 1))
  IKAM_AUTH=$ADMIN DEMO_USER=demo-admin DEMO_PW=$(cat "$ROOT/admin.pw") \
    $PY db "$B" "$ROOT/db-store" "$DBPORT" "$S/db/session.md"
  rm -f "$S/db/db-serve.log"
  put db "db: a recorded session"
fi
if [ -n "${ENWIK:-}" ] && [ ! -f "$ENWIK" ]; then echo "wiki: no dump at $ENWIK, skipped"; fi
if [ -n "${ENWIK:-}" ] && [ -f "$ENWIK" ] && fresh wiki; then
  readme wiki <<'MD'
# Wiki pages

The first pages of a Wikipedia dump (Hutter's enwik8), one file per page
exactly as its bytes stand, plus the dump's head. Joined in order they
rebuild the dump's first bytes exactly; `roundtrip.txt` has the check.
MD
  read -r SHA END NLINKS < <($PY wiki "$ENWIK" "${PAGES:-3000}" "$S/wiki") || { echo "wiki: split failed"; SHA=; }
  if [ -n "$SHA" ]; then
    put wiki "wiki: ${PAGES:-3000} pages"
    rm -rf "$ROOT/wiki-out"; "$B" --store "$ROOT/stores/wiki" checkout main "$ROOT/wiki-out" >/dev/null
    BACK=$($PY join "$ROOT/wiki-out"); rm -rf "$ROOT/wiki-out"
    printf 'pages: %s\nbytes: %s\nwiki links: %s\nsha256 of the first bytes: %s\nsha256 rebuilt from ikam: %s\n' \
      "${PAGES:-3000}" "$END" "$NLINKS" "$SHA" "$BACK" > "$S/wiki/roundtrip.txt"
    put wiki "wiki: round trip check"
    echo "wiki round trip: $([ "$SHA" = "$BACK" ] && echo exact || echo DIFFERS)"
  fi
fi
if fresh math; then
  readme math <<'MD'
# Math engine

ikam as a graph computation engine: tensors stored as content, op graphs as
graph objects, gradients and Hessian-vector products, pinned floats, stored
evaluation and solvers, on ikam's own foundations. Still on a branch; the
project's page links the pull request and the plan.
MD
  put math "math: what it is"
fi
for x in ${EXTRA:-}; do
  if fresh "$x"; then
    readme "$x" <<'MD'
# Links

This project holds links only; see its page in the explorer.
MD
    put "$x" "$x: links page"
  fi
done

echo "== fsck"
for d in "$ROOT"/stores/*/; do
  n=$(basename "$d")
  echo "fsck $n: $("$B" --store "$d" fsck 2>&1 | tail -1)"
done

echo "== config"
{
  echo "port $PORT"
  for h in $EXPLORER; do echo "explorer $h"; done
  for d in "$ROOT"/stores/*/; do
    n=$(basename "$d")
    echo "project $n $d"
    echo "user $n owner $ADMIN"
    echo "user $n owner $VIEWER"
    echo "max-body $n 25M"
  done
  cat <<CONF
title files Files
about files ikam as a filesystem: browse, edit, upload; every change a commit, copies kept once.
link files try log Changes, three commits
link files try stats What the store keeps
link files code $GH/blob/main/packages/kernel/src/bin/ikam/live.rs ikam live
title search Search and ask
about search Text search over ikam's documents and the passages that answer a question.
link search try search?q=Petri+net Search: Petri net
link search try search?q=scoped+share Search: scoped share
link search try ask?q=How+are+projects+kept+apart Ask: how are projects kept apart?
title lab Shares: lab
about lab The source side of a scoped share: pub/ is shared with partner, priv/ is not.
link lab try shares The shares lab has issued (owner only)
link lab code $GH/blob/main/docs/plans/2026-11-15-tenant-host.md The plan for separate projects shared by scope
title partner Shares: partner
about partner A separate project that reads lab's pub/ only through the share.
link partner shared from/lab/pub/files Files the share lets partner see
link partner shared from/lab/pub/read?path=pub/notes.md A note read through the share
link partner shared from/lab/pub/read?path=priv/plan.md A file outside the share (answers 404)
link partner shared from/lab/pub/search?q=a Search through the share
title nets Petri nets
about nets Work as Petri nets: a night of experiments planned on the loop net's own interpreter.
link nets try file?path=night-plan.txt The night plan
link nets code $GH/blob/main/packages/kernel/src/net.rs The net kernel
title loop Experiment loop
about loop The loop that runs ikam's experiments, driven one firing at a time by its net.
link loop try file?path=night.txt A night, firing by firing
link loop try file?path=net.txt The loop net
link loop pr $GH/pull/80 Self-improvement loop as configuration (pull request)
title mcp Agents (MCP)
about mcp ikam mcp: a store as tools for an AI agent.
link mcp try file?path=session.md A recorded session
link mcp code $GH/blob/main/packages/kernel/src/bin/ikam/mcp.rs ikam mcp
title db Database
about db ikam db serve: records for programs, every version kept.
link db try file?path=session.md A recorded session
link db code $GH/blob/main/packages/kernel/src/bin/ikam/dbserve.rs ikam db serve
title math Math engine
about math ikam as a graph computation engine: tensors, gradients, solvers and stored evaluation.
link math pr $GH/pull/104 The compute engine (pull request)
link math pr $GH/blob/exp/430-compute-graph/docs/plans/2026-11-26-math-engine.md The plan
CONF
  if [ -e "$ROOT/stores/wiki" ]; then
    cat <<CONF
title wiki Wiki pages
about wiki The first pages of a Wikipedia dump, one file per page, rebuilt byte for byte.
link wiki try file?path=roundtrip.txt The round trip check
link wiki try search?q=anarchism Search the pages
link wiki pr $GH/pull/82 Why a page tree ingests slowly (pull request)
CONF
  fi
  if [ -n "${LINKS:-}" ]; then cat "$LINKS"; fi
} > "$ROOT/host.conf"

echo "== serve"
nohup "$B" host --config "$ROOT/host.conf" "$PORT" >> "$ROOT/host.log" 2>&1 &
echo $! > "$ROOT/host.pid"
H="Host: ${EXPLORER%% *}"
U="http://127.0.0.1:$PORT"
code() { curl -s -o /dev/null -w '%{http_code}' -H "$H" "$@"; }
body() { curl -s -H "$H" "$@"; }
for _ in $(seq 100); do [ "$(code "$U/")" = 401 ] && break; sleep 0.2; done
N=(--netrc-file "$ROOT/.netrc")
curl -s -o /dev/null "${N[@]}" -H "$H" -H 'X-Ikam: 1' -X POST \
  "$U/p/lab/shares/issue?name=pub&to=partner&scope=path:pub&right=read&expires=31536000"

PASS=0; FAIL=0
check() {
  echo "$1: got $2, want $3"
  if [ "$2" = "$3" ]; then PASS=$((PASS + 1)); else FAIL=$((FAIL + 1)); fi
}
check list-needs-password "$(code "$U/")" 401
check list "$(code "${N[@]}" "$U/")" 200
LIST=$(body "${N[@]}" "$U/")
for d in "$ROOT"/stores/*/; do
  n=$(basename "$d")
  check "listed-$n" "$(grep -c "href=\"/projects/$n\"" <<< "$LIST")" 1
  check "page-$n" "$(code "${N[@]}" "$U/projects/$n")" 200
  check "files-$n" "$(body "${N[@]}" "$U/p/$n/" | grep -c "<base href=\"/p/$n/\">")" 1
done
check shared-read "$(body "${N[@]}" "$U/p/partner/from/lab/pub/read?path=pub/notes.md" | head -1)" "# Shared notes"
check outside-share "$(code "${N[@]}" "$U/p/partner/from/lab/pub/read?path=priv/plan.md")" 404
check search-finds "$(body "${N[@]}" "$U/p/search/search?q=Petri+net" | grep -q 'file?path=' && echo yes)" yes
check unknown-project "$(code "${N[@]}" "$U/p/nope/")" 401
echo "checks: $PASS passed, $FAIL failed"

if [ "${SERVE:-}" = 1 ]; then
  echo "serving: pid $(cat "$ROOT/host.pid"), 127.0.0.1:$PORT, explorer at $EXPLORER; log $ROOT/host.log"
else
  kill "$(cat "$ROOT/host.pid")"; rm -f "$ROOT/host.pid"; echo "stopped"
fi
[ "$FAIL" = 0 ]
