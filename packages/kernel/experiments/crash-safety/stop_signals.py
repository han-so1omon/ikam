"""E374: how `ikam db serve` and `ikam live` end on each stop signal, and
whether they keep working when their stderr goes away (the terminal that
started them closed, or a pipe's reader quit). For each case: did the
process exit, its exit code, whether it logged a line naming the stop, how
many of 5 requests it answered, and (live) whether a file written just
before the signal was committed. 127.0.0.1 only; synthetic data.
  python3 -I stop_signals.py IKAM_BIN WORK """
import base64, os, shutil, signal, socket, subprocess, sys, time, urllib.request

binary, work = sys.argv[1], sys.argv[2]
PW = "e374-test"
auth = subprocess.run([binary, "passwd", "u"], input=PW + "\n", capture_output=True,
                      text=True, check=True).stdout.strip().split("=", 1)[1]


def free_port():
    s = socket.socket(); s.bind(("127.0.0.1", 0)); p = s.getsockname()[1]; s.close(); return p


def req(port, method, path, body=None):
    r = urllib.request.Request("http://127.0.0.1:%d%s" % (port, path), data=body, method=method)
    r.add_header("Authorization", "Basic " + base64.b64encode(("u:" + PW).encode()).decode())
    with urllib.request.urlopen(r, timeout=5) as resp:
        return resp.status


def start(cmd, store, port, extra, pipe):
    args = [binary, "--store", store] + cmd + extra + [str(port)]
    err = subprocess.PIPE if pipe else open(store + ".log", "wb")
    p = subprocess.Popen(args, env=dict(os.environ, IKAM_AUTH=auth), stdout=subprocess.DEVNULL,
                         stderr=err)
    probe = "/db/t/head" if cmd[0] == "db" else "/"
    for _ in range(200):
        try:
            req(port, "GET", probe)
            return p, probe
        except Exception:
            time.sleep(0.1)
    raise RuntimeError("no start")


def case(name, cmd, extra, sig, close_err):
    d = os.path.join(work, name); shutil.rmtree(d, ignore_errors=True); os.makedirs(d)
    store, port = os.path.join(d, "store"), free_port()
    try:
        p, probe = start(cmd, store, port, extra, close_err)
    except RuntimeError:
        print({"case": name, "started": False}, flush=True)  # a flag this build lacks
        return
    if close_err:
        p.stderr.close()
    answered = 0
    for _ in range(5):
        try:
            answered += req(port, "GET", probe) == 200
        except Exception:
            pass
    wrote = None
    if cmd[0] == "live":
        try:
            wrote = req(port, "PUT", "/dav/note.txt", b"written just before the signal\n") in (200, 201, 204)
        except Exception:
            wrote = False
    p.send_signal(sig)
    try:
        code = p.wait(5)
    except subprocess.TimeoutExpired:
        code = None
        p.kill(); p.wait()
    line = ""
    if not close_err:
        log = open(store + ".log", "rb").read().decode(errors="replace").strip().splitlines()
        line = log[-1].split(" ", 1)[-1] if log else ""
    committed = None
    if cmd[0] == "live":
        ls = subprocess.run([binary, "--store", store, "ls", "main"], capture_output=True, text=True)
        committed = "note.txt" in ls.stdout
    print({"case": name, "exited": code is not None, "code": code, "last_line": line,
           "answered": answered, "file_written": wrote, "file_committed": committed}, flush=True)


DB = ["db", "serve", "--live"]
LIVE = ["live"]
case("db-sighup", DB, [], signal.SIGHUP, False)
case("db-sigterm", DB, [], signal.SIGTERM, False)
case("db-sighup-keep", DB, ["--keep-on-hangup"], signal.SIGHUP, False)
case("db-stderr-gone-sigterm", DB, [], signal.SIGTERM, True)
case("db-stderr-gone-log-requests", DB, ["--log-requests"], signal.SIGTERM, True)
case("live-sighup", LIVE, ["--commit-after", "60000"], signal.SIGHUP, False)
case("live-sigterm", LIVE, ["--commit-after", "60000"], signal.SIGTERM, False)
case("live-stderr-gone-sigterm", LIVE, ["--commit-after", "60000", "--log-requests"], signal.SIGTERM, True)
