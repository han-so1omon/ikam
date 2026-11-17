#!/bin/sh
# E342: what MLIR gives beyond speed, as probes; one JSON line each.
# Run from packages/kernel/experiments/kernel-jit. Needs MLIR_BIN (the
# folder of mlir-opt, mlir-translate, llc). Optional: IREE_VENV, a folder
# for a Python venv holding IREE's compiler and runtime (made if missing,
# with IREE_PYTHON, default python3); without it the IREE probes are
# skipped. Changes nothing outside the
# temporary folder and IREE_VENV.
set -u
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
j() { printf '{"exp":"E342","probe":"%s",%s}\n' "$1" "$2"; }
esc() { sed -e "s#$here#.#g" -e "s#$tmp#tmp#g" "$1" | tr '\n\t"\\' '    ' | tr -cd '[:print:]' | cut -c1-240; }
now() { python3 -c 'import time; print(int(time.time() * 1000))'; }
B=$MLIR_BIN

# 1. One module, several CPU targets and WebAssembly: lower once, then llc.
if cargo run -q --release -- lower "$tmp/k.ll" 2> "$tmp/err"; then
  for t in aarch64-apple-darwin x86_64-unknown-linux-gnu riscv64-unknown-linux-gnu wasm32-unknown-unknown; do
    if "$B/llc" -O3 -mtriple=$t -filetype=obj "$tmp/k.ll" -o "$tmp/k.o" 2> "$tmp/err"; then
      j target "\"triple\":\"$t\",\"ok\":true,\"object_bytes\":$(wc -c < "$tmp/k.o" | tr -d ' ')"
    else
      j target "\"triple\":\"$t\",\"ok\":false,\"error\":\"$(esc "$tmp/err")\""
    fi
  done
else
  j lower "\"ok\":false,\"error\":\"$(esc "$tmp/err")\""
fi

# 2. A dialect of ikam's own from IRDL, loaded at run time.
r() { "$B/mlir-opt" "$@" > /dev/null 2> "$tmp/err" && echo true || echo false; }
j irdl "\"without_irdl_parses\":$(r --allow-unregistered-dialect=false "$here/ikam-use.mlir"),\"with_irdl_parses\":$(r --irdl-file="$here/ikam.irdl.mlir" "$here/ikam-use.mlir"),\"bad_result_type_accepted\":$(r --irdl-file="$here/ikam.irdl.mlir" "$here/ikam-bad.mlir")"
"$B/mlir-opt" --irdl-file="$here/ikam.irdl.mlir" "$here/ikam-use.mlir" > /dev/null 2> "$tmp/err" \
  || j irdl_error "\"error\":\"$(esc "$tmp/err")\""

# 3. What the tool chain weighs.
j toolchain "\"mlir_env_kb\":$(du -sk "$B/.." | cut -f1),\"mlir_opt\":\"$("$B/mlir-opt" --version | grep -i version | head -1 | tr -d '"')\""

# 4. IREE: the linalg file for the CPU and for Metal, then benchmarked.
[ -n "${IREE_VENV:-}" ] || { j iree '"skipped":"IREE_VENV not set"'; exit 0; }
[ -x "$IREE_VENV/bin/pip" ] || "${IREE_PYTHON:-python3}" -m venv "$IREE_VENV"
"$IREE_VENV/bin/pip" install -q iree-base-compiler iree-base-runtime > "$tmp/err" 2>&1 \
  || { j iree "\"install\":false,\"error\":\"$(esc "$tmp/err")\""; exit 0; }
v=$("$IREE_VENV/bin/pip" show iree-base-compiler | grep '^Version' | cut -d' ' -f2)
j iree "\"install\":true,\"version\":\"$v\",\"venv_kb\":$(du -sk "$IREE_VENV" | cut -f1)"
C=$IREE_VENV/bin/iree-compile
for target in cpu metal; do
  if [ $target = cpu ]; then
    new="--iree-hal-target-device=local --iree-hal-local-target-device-backends=llvm-cpu --iree-llvmcpu-target-cpu=host"
    old="--iree-hal-target-backends=llvm-cpu --iree-llvmcpu-target-cpu=host"
    dev=local-task
  else
    new="--iree-hal-target-device=metal"
    old="--iree-hal-target-backends=metal-spirv"
    dev=metal
  fi
  t0=$(now)
  if "$C" $new "$here/scores.mlir" -o "$tmp/$target.vmfb" 2> "$tmp/err" || "$C" $old "$here/scores.mlir" -o "$tmp/$target.vmfb" 2> "$tmp/err"; then
    ms=$(( $(now) - t0 ))
    for f in scores scan; do
      if [ $f = scores ]; then in="--input=31600x256xf32=0.01 --input=256x316xf32=0.01"; else in="--input=200000x256xf32=0.01 --input=256xf32=0.01"; fi
      "$IREE_VENV/bin/iree-benchmark-module" --module="$tmp/$target.vmfb" --device=$dev --function=$f $in \
        --benchmark_repetitions=5 > "$tmp/out" 2> "$tmp/err" || cat "$tmp/err" >> "$tmp/out"
      grep -E 'mean|_mean|BM_' "$tmp/out" > "$tmp/bm" || cp "$tmp/out" "$tmp/bm"
      j iree_run "\"target\":\"$target\",\"function\":\"$f\",\"compile_ms\":$ms,\"benchmark\":\"$(esc "$tmp/bm")\""
    done
  else
    j iree_run "\"target\":\"$target\",\"compile\":false,\"error\":\"$(esc "$tmp/err")\""
  fi
done
