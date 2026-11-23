"""E377: the same workload against `ikam db serve` and a throwaway PostgreSQL,
for A/B comparisons (PostgreSQL as the yardstick, not as a store to keep).

Both sides get the same synthetic rows (a primary key, a date, a word, a
number and padding; one secondary index on the word) and the same steps:

  load      ROWS rows in 2,000-row batches, each batch committed before the
            next is sent (ikam: one append; PostgreSQL: one transaction)
  get       2,000 point reads by primary key
  by_word   300 lookups of every row with a given word (secondary index)
  range     300 reads of 100 rows in key order from a random key
  count     the number of rows, 20 times
  update    1% of rows given a new value, in 2,000-row batches
  search    50 ranked text queries (ikam: BM25; PostgreSQL: full text with a
            GIN index and ts_rank; the rankings are not the same method)
  restart   stop, start, and time the first point read and the first search
  size      store directory (ikam) and data directory (PostgreSQL), MB

Then it checks that both hold the same rows (count, and every value of a
sample of 1,000 keys). Each client keeps one connection open. PostgreSQL
runs from its own data directory under WORK on a Unix socket only, in a
directory only its owner can enter (0700), with its defaults (synchronous
commit on). Run as root, it runs PostgreSQL as the `postgres` user (runuser);
run as anyone else, as that user. Nothing leaves the machine. Linux; needs
psycopg2 and the PostgreSQL server binaries.

  python3 -I ab_bench.py IKAM_BIN PG_BIN_DIR WORK ROWS "IKAM FLAGS" [view]

With `view` (E379) ikam keeps no index entries of its own: rows are JSON
objects, `db serve` is started with `--views` and an index on the word
(E361), and a lookup by word is one `view?name=by_word&eq=W` request. The
first lookup builds the view; its time is reported apart. """
import base64, http.client, json, os, random, shutil, socket, struct, subprocess, sys, time

import psycopg2

binary, pg_bin, work, rows, flags = (sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4]),
                                     sys.argv[5].split())
VIEW = sys.argv[6:7] == ["view"]
BATCH = 2000
PW = "e377-test"
WORDS = 5000
rnd = random.Random(377)


def row(i, version=0):
    r = random.Random(i * 7 + version)
    word = "word%d" % r.randrange(WORDS)
    date = "2026-01-%02d" % (1 + i % 28)
    pad = " ".join("w%d" % r.randrange(20000) for _ in range(r.randrange(4, 14)))
    return "p%019d" % i, date, word, round(r.random() * 1000, 3), pad


def ms_stats(xs):
    xs = sorted(xs)
    return {"p50_ms": round(xs[len(xs) // 2] * 1e3, 2),
            "p99_ms": round(xs[max(0, int(len(xs) * 0.99) - 1)] * 1e3, 2)}


def du(path):
    return sum(os.path.getsize(os.path.join(r, f)) for r, _, fs in os.walk(path) for f in fs
               if os.path.exists(os.path.join(r, f)))


# ---- ikam -------------------------------------------------------------------

def pack(*parts):
    return b"".join(struct.pack(">I", len(p)) + p for p in parts)


def unframe(b):
    out, i = [], 0
    while i < len(b):
        n = struct.unpack(">I", b[i:i + 4])[0]; k = b[i + 4:i + 4 + n]; i += 4 + n
        n = struct.unpack(">I", b[i:i + 4])[0]; v = b[i + 4:i + 4 + n]; i += 4 + n
        out.append((k.decode(), v))
    return out


class Ikam:
    name = "ikam"

    def __init__(self, store):
        self.store = store
        s = socket.socket(); s.bind(("127.0.0.1", 0)); self.port = s.getsockname()[1]; s.close()
        line = subprocess.run([binary, "passwd", "u"], input=PW + "\n", capture_output=True,
                              text=True, check=True).stdout.strip()
        self.auth = line.split("=", 1)[1]
        self.hdr = {"Authorization": "Basic " + base64.b64encode(("u:" + PW).encode()).decode()}
        self.seq = 0
        self.extra = []
        if VIEW:
            vf = store + ".views"
            os.makedirs(os.path.dirname(vf), exist_ok=True)
            open(vf, "w").write("by_word index t/r/ by=field:word\n")
            self.extra = ["--views", vf]

    def start(self):
        self.log = open(self.store + ".log", "ab")
        self.proc = subprocess.Popen([binary, "--store", self.store, "db", "serve", *flags, *self.extra,
                                      str(self.port)], env=dict(os.environ, IKAM_AUTH=self.auth),
                                     stdout=self.log, stderr=self.log)
        for _ in range(600):
            try:
                self.conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=1800)
                self.call("GET", "/db/t/head")
                return
            except (ConnectionError, OSError):
                time.sleep(0.1)
        raise RuntimeError("ikam did not start")

    def stop(self):
        self.conn.close(); self.proc.terminate(); self.proc.wait(120); self.log.close()

    def call(self, method, path, body=None):
        h = dict(self.hdr)
        if body is not None:
            h["Content-Type"] = "application/x-ikam-events"
        self.conn.request(method, path, body=body, headers=h)
        r = self.conn.getresponse()
        data = r.read()
        if r.status not in (200, 404):
            raise RuntimeError("%s %s: %d %s" % (method, path, r.status, data[:200]))
        return r.status, data

    def write(self, rs):
        out = []
        for pk, date, word, num, pad in rs:
            self.seq += 1
            val = json.dumps([date, word, num, pad]).encode()
            out.append(b"\x00" + pack(("t/r/" + pk).encode(), b"e%d" % self.seq, val))
        self.call("POST", "/db/t/append", b"".join(out))

    @staticmethod
    def value(date, word, num, pad):
        if VIEW:
            return json.dumps({"date": date, "word": word, "num": num, "pad": pad}).encode()
        return json.dumps([date, word, num, pad]).encode()

    @staticmethod
    def plain(v):
        # A row's value as the PostgreSQL side gives it, for the check.
        if v is None or not VIEW:
            return v
        o = json.loads(v)
        return json.dumps([o["date"], o["word"], o["num"], o["pad"]]).encode()

    def load(self, rs):
        # Rows and their index entries, as a client of a key-value store does
        # (with `view`, the server keeps the index).
        out = []
        for pk, date, word, num, pad in rs:
            self.seq += 1
            val = self.value(date, word, num, pad)
            out.append(b"\x00" + pack(("t/r/" + pk).encode(), b"e%d" % self.seq, val))
            if VIEW:
                continue
            out.append(b"\x00" + pack(("i/w/%s/%s" % (word, pk)).encode(), b"i%d" % self.seq, b""))
        self.call("POST", "/db/t/append", b"".join(out))

    def update(self, rs, old_words):
        out = []
        for (pk, date, word, num, pad), old in zip(rs, old_words):
            self.seq += 1
            val = self.value(date, word, num, pad)
            out.append(b"\x00" + pack(("t/r/" + pk).encode(), b"e%d" % self.seq, val))
            if old != word and not VIEW:
                out.append(b"\x01" + pack(("i/w/%s/%s" % (old, pk)).encode(), b"d%d" % self.seq))
                out.append(b"\x00" + pack(("i/w/%s/%s" % (word, pk)).encode(), b"i%d" % self.seq, b""))
        self.call("POST", "/db/t/append", b"".join(out))

    def get(self, pk):
        st, v = self.call("GET", "/db/t/get?key=t/r/" + pk)
        return self.plain(v) if st == 200 else None

    def by_word(self, word):
        if VIEW:
            _, b = self.call("GET", "/db/t/view?name=by_word&eq=" + word)
            return [v for _, v in unframe(b)]
        _, b = self.call("GET", "/db/t/scan?prefix=i/w/%s/" % word)
        keys = ["t/r/" + k.rsplit("/", 1)[1] for k, _ in unframe(b)]
        if not keys:
            return []
        conn = self.conn
        conn.request("POST", "/db/t/get", body="\n".join(keys).encode(), headers=self.hdr)
        r = conn.getresponse(); data = r.read()
        return [v for _, v in unframe(data)]

    def range(self, pk, n):
        _, b = self.call("GET", "/db/t/scan?start=t/r/%s&end=t/r/q&limit=%d" % (pk, n))
        return unframe(b)

    def count(self):
        _, b = self.call("GET", "/db/t/count?prefix=t/r/")
        return json.loads(b)["count"]

    def search(self, q):
        _, b = self.call("GET", "/db/t/search?prefix=t/r/&k=10&q=" + q)
        return json.loads(b)

    def size(self):
        return du(self.store)


# ---- PostgreSQL ---------------------------------------------------------------

class Pg:
    name = "postgres"

    def __init__(self, data):
        self.data, self.sock = data, os.path.dirname(data)
        os.makedirs(self.sock, exist_ok=True)
        # initdb's trust login is for whoever can reach the socket: keep its
        # directory to its owner.
        os.chmod(self.sock, 0o700)
        if os.geteuid() == 0:  # PostgreSQL won't run as root
            shutil.chown(self.sock, "postgres", "postgres")
        self.port = 5400 + rnd.randrange(500)
        self.su(["initdb", "-D", data, "-A", "trust", "-U", "postgres", "--no-instructions"])

    def su(self, cmd):
        pre = ["runuser", "-u", "postgres", "--"] if os.geteuid() == 0 else []
        subprocess.run(pre + [os.path.join(pg_bin, cmd[0])] + cmd[1:], check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def start(self):
        self.su(["pg_ctl", "-D", self.data, "-w", "-l", self.data + ".log", "-o",
                 "-c listen_addresses='' -c unix_socket_directories=%s"
                 " -c unix_socket_permissions=0700 -p %d" % (self.sock, self.port),
                 "start"])
        self.conn = psycopg2.connect(host=self.sock, port=self.port, user="postgres",
                                     dbname="postgres")
        self.cur = self.conn.cursor()

    def stop(self):
        self.conn.close()
        self.su(["pg_ctl", "-D", self.data, "-w", "-m", "fast", "stop"])

    def setup(self):
        self.cur.execute("CREATE TABLE r (pk text PRIMARY KEY, date text, word text, num float8,"
                         " pad text)")
        self.cur.execute("CREATE INDEX r_word ON r (word)")
        self.cur.execute("CREATE INDEX r_text ON r USING gin (to_tsvector('simple', word || ' ' || pad))")
        self.conn.commit()

    def load(self, rs):
        from psycopg2.extras import execute_values
        execute_values(self.cur, "INSERT INTO r VALUES %s", rs, page_size=BATCH)
        self.conn.commit()

    def update(self, rs, _old):
        from psycopg2.extras import execute_values
        execute_values(self.cur, "INSERT INTO r VALUES %s ON CONFLICT (pk) DO UPDATE SET"
                       " date = EXCLUDED.date, word = EXCLUDED.word, num = EXCLUDED.num,"
                       " pad = EXCLUDED.pad", rs, page_size=BATCH)
        self.conn.commit()

    def _one(self, sql, args):
        self.cur.execute(sql, args)
        out = self.cur.fetchall()
        self.conn.commit()
        return out

    def get(self, pk):
        r = self._one("SELECT date, word, num, pad FROM r WHERE pk = %s", (pk,))
        return json.dumps(list(r[0])).encode() if r else None

    def by_word(self, word):
        return self._one("SELECT date, word, num, pad FROM r WHERE word = %s", (word,))

    def range(self, pk, n):
        return self._one("SELECT pk, date, word, num, pad FROM r WHERE pk >= %s ORDER BY pk"
                         " LIMIT %s", (pk, n))

    def count(self):
        return self._one("SELECT count(*) FROM r", ())[0][0]

    def search(self, q):
        return self._one("SELECT pk, ts_rank(to_tsvector('simple', word || ' ' || pad), q) AS s"
                         " FROM r, plainto_tsquery('simple', %s) q WHERE to_tsvector('simple',"
                         " word || ' ' || pad) @@ q ORDER BY s DESC LIMIT 10", (q,))

    def size(self):
        return du(self.data)


def timed(f, args):
    out = []
    for a in args:
        t = time.perf_counter(); f(*a); out.append(time.perf_counter() - t)
    return out


def run(side):
    res = {"side": side.name}
    side.start()
    if isinstance(side, Pg):
        side.setup()
    t0 = time.time()
    lat = []
    for s in range(0, rows, BATCH):
        b = [row(i) for i in range(s, min(rows, s + BATCH))]
        t = time.perf_counter(); side.load(b); lat.append(time.perf_counter() - t)
    res["load"] = dict(s=round(time.time() - t0, 1), **ms_stats(lat))
    r = random.Random(1)
    res["get"] = ms_stats(timed(side.get, [(row(r.randrange(rows))[0],) for _ in range(2000)]))
    lat = timed(side.by_word, [("word%d" % r.randrange(WORDS),) for _ in range(301)])
    res["by_word"] = dict(first_ms=round(lat[0] * 1e3, 1), **ms_stats(lat[1:]))
    if isinstance(side, Ikam) and VIEW:
        # The index alone (keys, no row values), to split the lookup's time;
        # its own random words, so both sides still update the same rows.
        res["by_word_keys_only"] = ms_stats(timed(side.call, [
            ("GET", "/db/t/view?name=by_word&keys=1&eq=word%d" % k.randrange(WORDS))
            for k in [random.Random(3)] for _ in range(300)]))
    res["range"] = ms_stats(timed(side.range, [(row(r.randrange(rows))[0], 100)
                                               for _ in range(300)]))
    res["count"] = ms_stats(timed(side.count, [()] * 20))
    changed = sorted(r.sample(range(rows), rows // 100))
    t0 = time.time()
    for s in range(0, len(changed), BATCH):
        ids = changed[s:s + BATCH]
        side.update([row(i, 1) for i in ids], [row(i)[2] for i in ids])
    res["update_s"] = round(time.time() - t0, 1)
    qs = [("w%d" % r.randrange(20000),) for _ in range(50)]
    lat = timed(side.search, qs)
    res["search"] = dict(first_ms=round(lat[0] * 1e3, 1), **ms_stats(lat[1:]))
    side.stop()
    t = time.time(); side.start(); res["restart_s"] = round(time.time() - t, 2)
    res["first_get_ms"] = round(timed(side.get, [(row(5)[0],)])[0] * 1e3, 1)
    res["first_search_ms"] = round(timed(side.search, [("w7",)])[0] * 1e3, 1)
    res["first_by_word_ms"] = round(timed(side.by_word, [("word7",)])[0] * 1e3, 1)
    res["by_word_rows"] = [len(side.by_word("word%d" % w)) for w in (7, 42, 4999)]
    res["size_mb"] = round(side.size() / 1e6, 1)
    sample = [row(i)[0] for i in random.Random(2).sample(range(rows), 1000)]
    res["_check"] = (side.count(), [side.get(pk) for pk in sample])
    side.stop()
    return res


os.makedirs(work, exist_ok=True)
for d in ("ikam", "pg"):
    shutil.rmtree(os.path.join(work, d), ignore_errors=True)
os.makedirs(os.path.join(work, "ikam"))
a = run(Ikam(os.path.join(work, "ikam", "store")))
b = run(Pg(os.path.join(work, "pg", "data")))
ca, cb = a.pop("_check"), b.pop("_check")
norm = lambda v: v and json.loads(v)
same = ca[0] == cb[0] and all(norm(x) == norm(y) for x, y in zip(ca[1], cb[1]))
same = same and a["by_word_rows"] == b["by_word_rows"]
print(json.dumps({"rows": rows, "ikam_flags": " ".join(flags), "view": VIEW, "same_rows": same,
                  "count": [ca[0], cb[0]], "ikam": a, "postgres": b}), flush=True)
