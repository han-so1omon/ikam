"""E367: load ROWS with the first flag set, then restart once per later set,
timing the first and second write after each start (TRACE=1 samples stacks
with gdb). Usage: restart_first_write.py IKAM_BIN WORK ROWS "FLAGS;FLAGS;..."."""
import json, os, shutil, subprocess, sys, threading, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import db_first_write as d

binary, work, rows, sets = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4].split(";")
load_flags = sets[0].split()
batch = 2000
shutil.rmtree(work, ignore_errors=True); os.makedirs(work)
store = os.path.join(work, "store")
s = d.Server(binary, store, load_flags)
s.start()
t0 = time.time()
for start in range(0, rows, batch):
    s.append(d.events(b"big", start, min(batch, rows - start), b"load"), timeout=600)
print(json.dumps({"load_s": round(time.time() - t0, 1), "flags": sets[0]}), flush=True)
s.stop()

def sample(pid, stop, out):
    n = 0
    while not stop.is_set():
        r = subprocess.run(["gdb", "-p", str(pid), "-batch", "-ex", "thread apply all bt 40"],
                           capture_output=True, text=True)
        out.write(f"=== sample {n} t={time.time():.2f}\n{r.stdout}\n"); n += 1
        time.sleep(0.15)

for k, fl in enumerate(sets[1:]):
    s.flags = fl.split()
    s.start()
    trace = os.environ.get("TRACE") == "1"
    stop = threading.Event()
    if trace:
        f = open(os.path.join(work, f"stacks-{k}.txt"), "w")
        th = threading.Thread(target=sample, args=(s.proc.pid, stop, f)); th.start()
    first, ok = s.append(d.events(b"new%d" % k, 0, batch, b"after%d" % k))
    stop.set()
    if trace: th.join(); f.close()
    nxt, _ = s.append(d.events(b"new%d" % k, batch, batch, b"after%d" % k))
    s.stop()
    print(json.dumps({"restart": k, "flags": fl, "first_write_s": round(first, 3), "ok": ok, "second_write_s": round(nxt, 3)}), flush=True)
