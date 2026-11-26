"""E399: the store `view_edit.py` copies, without the PostgreSQL side of
E379's harness: ROWS rows of `ab_bench.py`'s generator (JSON objects with a
date, a word of 5,000, a number and padding) appended in 2,000-row batches
to `db serve --live --views VIEWS` (one index on the word), then the server
stopped. Synthetic rows; 127.0.0.1.
  python3 -I make_store.py IKAM_BIN STORE ROWS """
import base64, http.client, json, os, random, shutil, struct, subprocess, sys, time

b, store, rows = sys.argv[1], sys.argv[2], int(sys.argv[3])
PW = "p"
auth = subprocess.run([b, "passwd", "u"], input=PW + "\n", capture_output=True, text=True, check=True).stdout.strip().split("=", 1)[1]
h = {"Authorization": "Basic " + base64.b64encode(("u:" + PW).encode()).decode()}
pack = lambda *ps: b"".join(struct.pack(">I", len(p)) + p for p in ps)


def row(i):
    # ab_bench.py's row(i) with view-mode values.
    r = random.Random(i * 7)
    word = "word%d" % r.randrange(5000)
    date = "2026-01-%02d" % (1 + i % 28)
    pad = " ".join("w%d" % r.randrange(20000) for _ in range(r.randrange(4, 14)))
    return "p%019d" % i, json.dumps({"date": date, "word": word, "num": round(r.random() * 1000, 3), "pad": pad}).encode()


shutil.rmtree(store, ignore_errors=True)
views = store + ".views"
open(views, "w").write("by_word index t/r/ by=field:word\n")
p = subprocess.Popen([b, "--store", store, "db", "serve", "--live", "--views", views, "18557"],
                     env=dict(os.environ, IKAM_AUTH=auth), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
for _ in range(300):
    try:
        c = http.client.HTTPConnection("127.0.0.1", 18557, timeout=600)
        c.request("GET", "/db/t/head", headers=h)
        c.getresponse().read()
        break
    except OSError:
        time.sleep(0.1)
t = time.perf_counter()
seq = 0
for start in range(0, rows, 2000):
    out = []
    for i in range(start, min(rows, start + 2000)):
        seq += 1
        pk, val = row(i)
        out.append(b"\x00" + pack(("t/r/" + pk).encode(), b"e%d" % seq, val))
    hh = dict(h, **{"Content-Type": "application/x-ikam-events"})
    c.request("POST", "/db/t/append", body=b"".join(out), headers=hh)
    r = c.getresponse()
    assert r.status == 200, r.read()[:200]
    r.read()
p.terminate()
p.wait()
print(json.dumps({"rows": rows, "load_s": round(time.perf_counter() - t, 1), "views": views}))
