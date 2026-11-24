"""E386: kill the night runner at random points and check what it reported.

Each call proposes one id; a call is killed with SIGKILL after a random
delay. A call counts as acknowledged when it exited 0 and printed its
commit. Afterwards every acknowledged id must be in the night's commits,
the replay must equal the marking and fsck must be clean.

    python3 -I kill_runner.py LOOP_DRIVE_BIN STORE_DIR CALLS MAX_DELAY_MS
"""
import json, random, subprocess, sys, time

bin_, store, calls, max_ms = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
random.seed(386)
acked, killed, times = [], 0, []
for i in range(calls):
    tok = f"E{1000 + i}"
    t0 = time.monotonic()
    p = subprocess.Popen([bin_, store, "kill", "plan", tok], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    try:
        out, _ = p.communicate(timeout=random.uniform(0.0, max_ms) / 1000)
        if p.returncode == 0 and out.startswith(b"plan"):
            acked.append(tok)
            times.append(time.monotonic() - t0)
    except subprocess.TimeoutExpired:
        p.kill()
        p.wait()
        killed += 1
out = subprocess.run([bin_, store, "kill", "check"], capture_output=True, text=True)
logged = {l.split()[1] for l in out.stdout.splitlines()[1:] if l.strip().startswith("plan")}
print(json.dumps({
    "calls": calls, "acked": len(acked), "killed": killed,
    "acked_missing": sum(t not in logged for t in acked),
    "committed_not_acked": len(logged - set(acked)),
    "check_ok": out.returncode == 0, "check": out.stdout.splitlines()[0] if out.stdout else out.stderr[-300:],
    "call_p50_ms": round(sorted(times)[len(times) // 2] * 1000, 1) if times else None,
}))
