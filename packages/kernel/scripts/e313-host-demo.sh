#!/bin/bash
# E313 and E314 on a real machine: `ikam host` with three made-up test
# projects, checked over HTTP on 127.0.0.1, then removed.
#
# Safe by design: it runs as you, needs no sudo, writes only under $ROOT
# (and the cargo target dir), serves on 127.0.0.1:$PORT only for the run,
# and deletes $ROOT at the end. The repository is only read.
#
#   ROOT=/path/to/empty-folder bash scripts/e313-host-demo.sh
#
# PORT (default 18750) must be free. Prints the test results, one line per
# check ("name: got X, want Y"), a timing line, the stop and fsck lines.
# Passwords are random, kept in $ROOT for the run, and never printed.
set -uo pipefail

K=$(cd "$(dirname "$0")/.." && pwd)    # packages/kernel of this checkout
ROOT=${ROOT:?set ROOT to a new folder for the run}
PORT=${PORT:-18750}
U="http://127.0.0.1:$PORT"

if [ -e "$ROOT" ] && [ -n "$(ls -A "$ROOT" 2>/dev/null)" ]; then
  echo "preflight: $ROOT is not empty; give a new folder"; exit 1
fi
if command -v lsof >/dev/null && lsof -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; then
  echo "preflight: port $PORT is in use; set PORT to a free one"; exit 1
fi
mkdir -p "$ROOT/stores"

echo "== build and tests"
(cd "$K" && cargo build --release -q) || { echo "build: failed"; exit 1; }
(cd "$K" && cargo test --release -q --test share 2>&1 | grep "test result" | sed 's/^/share tests: /')
(cd "$K" && cargo test --release -q --bin ikam host:: 2>&1 | grep "test result" | sed 's/^/host tests: /')
(cd "$K" && cargo test --release -q --bin ikam host::tests::overhead -- --ignored --nocapture 2>&1 | grep "^round")
B="${CARGO_TARGET_DIR:-$K/target}/release/ikam"
[ -x "$B" ] || B="$K/target/release/ikam"
[ -x "$B" ] || { echo "build: no ikam binary found"; exit 1; }

echo "== demo"
pw() { head -c 18 /dev/urandom | base64 | tr -d '/+=' ; }
mk() { printf '%s\n' "$2" | "$B" passwd "$1" 2>/dev/null | tail -1 | sed 's/^IKAM_AUTH=//'; }
PA=$(pw); PB=$(pw); PR=$(pw); PC=$(pw)
umask 077
cat > "$ROOT/host.conf" <<CONF
project alpha $ROOT/stores/alpha
user alpha owner $(mk alpha-owner "$PA")
project beta $ROOT/stores/beta
user beta owner $(mk beta-owner "$PB")
user beta read $(mk beta-reader "$PR")
project gamma $ROOT/stores/gamma
user gamma owner $(mk gamma-owner "$PC")
max-body gamma 1K
CONF
"$B" host --config "$ROOT/host.conf" "$PORT" 2>"$ROOT/host.log" & HOST=$!
for _ in $(seq 50); do curl -s -o /dev/null "$U/" && break; sleep 0.2; done
code() { curl -s -o /dev/null -w '%{http_code}' "$@"; }
body() { curl -s "$@"; }
A=(-u "alpha-owner:$PA" -H 'Host: alpha.localhost')
Bo=(-u "beta-owner:$PB" -H 'Host: beta.localhost')
Br=(-u "beta-reader:$PR" -H 'Host: beta.localhost')
G=(-u "gamma-owner:$PC" -H 'Host: gamma.localhost')
X=(-H 'X-Ikam: 1' -X POST)
PASS=0; FAIL=0
check() {
  echo "$1: got $2, want $3"
  if [ "$2" = "$3" ]; then PASS=$((PASS + 1)); else FAIL=$((FAIL + 1)); fi
}
check mkcol-pub "$(code "${A[@]}" -X MKCOL "$U/dav/pub")" 201
check mkcol-priv "$(code "${A[@]}" -X MKCOL "$U/dav/priv")" 201
check put-pub "$(code "${A[@]}" -X PUT --data-binary 'hello from alpha' "$U/dav/pub/note.md")" 201
check put-priv "$(code "${A[@]}" -X PUT --data-binary 'alpha only' "$U/dav/priv/secret.md")" 201
check beta-owner-on-alpha "$(code -u "beta-owner:$PB" -H 'Host: alpha.localhost' "$U/dav/pub/note.md")" 401
check wrong-password "$(code -u 'alpha-owner:nope' -H 'Host: alpha.localhost' "$U/dav/")" 401
check unknown-host "$(code -u "alpha-owner:$PA" -H 'Host: delta.localhost' "$U/dav/")" 404
check beta-sees-no-alpha-file "$(code "${Bo[@]}" "$U/dav/pub/note.md")" 404
check beta-reader-cannot-write "$(code "${Br[@]}" -X PUT --data-binary x "$U/dav/x.md")" 403
check gamma-body-limit "$(code "${G[@]}" -X PUT --data-binary "$(head -c 2048 /dev/zero | tr '\0' x)" "$U/dav/big")" 413
check issue-share "$(code "${A[@]}" "${X[@]}" "$U/shares/issue?name=pub&to=beta&scope=path:pub&right=merge&expires=600")" 200
check read-in-scope "$(body "${Br[@]}" "$U/from/alpha/pub/read?path=pub/note.md")" "hello from alpha"
check read-outside-scope "$(code "${Bo[@]}" "$U/from/alpha/pub/read?path=priv/secret.md")" 404
check read-dotdot-encoded "$(code "${Bo[@]}" "$U/from/alpha/pub/read?path=pub%2F..%2Fpriv%2Fsecret.md")" 404
check gamma-through-beta-share "$(code "${G[@]}" "$U/from/alpha/pub/files")" 404
check reader-cannot-merge "$(code "${Br[@]}" "${X[@]}" "$U/from/alpha/pub/merge")" 403
check owner-merges "$(body "${Bo[@]}" "${X[@]}" "$U/from/alpha/pub/merge" | cut -c1-9)" "merged as"
check merged-copy-in-beta "$(body "${Bo[@]}" "$U/dav/pub/note.md")" "hello from alpha"
check private-not-merged "$(code "${Bo[@]}" "$U/dav/priv/secret.md")" 404
check revoke "$(code "${A[@]}" "${X[@]}" "$U/shares/revoke?name=pub")" 200
check read-after-revoke "$(code "${Bo[@]}" "$U/from/alpha/pub/files")" 404
check copy-stays-after-revoke "$(body "${Bo[@]}" "$U/dav/pub/note.md")" "hello from alpha"
for _ in $(seq 300); do
  curl -s -o /dev/null -w '%{time_total}\n' "${A[@]}" "$U/dav/pub/note.md"
done | sort -n > "$ROOT/times.txt"
echo "timing: 300 reads over HTTP, p50 $(sed -n 150p "$ROOT/times.txt") s, p99 $(sed -n 297p "$ROOT/times.txt") s"
kill -TERM $HOST; wait $HOST 2>/dev/null
echo "stop: $(tail -1 "$ROOT/host.log" | cut -d' ' -f2-)"
echo "fsck beta: $("$B" --store "$ROOT/stores/beta" fsck 2>&1 | tail -1)"
if [ "$FAIL" -gt 0 ]; then
  echo "== host log (checks failed)"
  sed "s|$ROOT|ROOT|g" "$ROOT/host.log" | tail -20
fi
echo "checks: $PASS passed, $FAIL failed"
rm -rf "$ROOT" && echo "cleanup: removed the run folder"
