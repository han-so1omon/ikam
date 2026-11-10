"""E165: TableIR stored in the kernel, json and md rendered on read.

E161 lifted each workbook's csv into a typed TableIR (cell types from the
csv's quoting, plus residuals: the md's title and source line, and the
csv's empty rows) and rendered csv, json and md byte for byte. Here the IR
is what is stored: the csv itself (it carries the cell types) plus a small
sidecar with the residuals, ingested by the kernel (`ikam ingest`: commit,
repack, verify). Compared with ingesting the three text formats as files,
and with all six formats (xlsx, docx and pdf kept as files either way).

Per layout: the kernel's stored bytes. Per table: json and md rendered from
the stored csv and sidecar (read back with `ikam cat`), checked byte-exact,
and the render time. A scaled run (E161's synthetic 10,000 rows) times
rendering on read against reading the stored file.

Run from packages/kernel after `cargo build --release`:
    python3 scripts/table_ir_store.py OUT.jsonl
"""

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from table_ir import FIX, IKAM, TableIR  # noqa: E402


def sidecar(ir):
    return json.dumps({"title": ir.title, "source": ir.source, "blank": ir.blank},
                      ensure_ascii=False, separators=(",", ":")).encode() + b"\n"


def from_sidecar(csv_text, side):
    s = json.loads(side)
    md = f"# {s['title']}\n\n{s['source']}\n"
    ir = TableIR.lift(csv_text, md)
    return ir


def ingest(d, store):
    out = subprocess.run([IKAM, "--store", store, "ingest", d], capture_output=True,
                         text=True, check=True).stdout
    m = re.search(r"stored_bytes=(\d+).*fsck_failures=(\d+)", out)
    return int(m.group(1)), int(m.group(2))


def cat(store, rev_path):
    return subprocess.run([IKAM, "--store", store, "cat", rev_path], capture_output=True,
                          check=True).stdout


def main(out_path):
    work = tempfile.mkdtemp()
    out = open(out_path, "w")
    names = sorted(n[:-4] for n in os.listdir(FIX) if n.endswith(".csv"))
    layouts = {k: os.path.join(work, k) for k in ("text3", "ir", "all6", "ir+office")}
    for d in layouts.values():
        os.makedirs(d)
    for n in names:
        txt = {f: open(os.path.join(FIX, f"{n}.{f}"), "rb").read() for f in
               ("csv", "json", "md", "xlsx", "docx", "pdf")}
        ir = TableIR.lift(txt["csv"].decode(), txt["md"].decode())
        for f in ("csv", "json", "md"):
            open(os.path.join(layouts["text3"], f"{n}.{f}"), "wb").write(txt[f])
        for f in txt:
            open(os.path.join(layouts["all6"], f"{n}.{f}"), "wb").write(txt[f])
        for d in (layouts["ir"], layouts["ir+office"]):
            open(os.path.join(d, f"{n}.csv"), "wb").write(txt["csv"])
            open(os.path.join(d, f"{n}.table.json"), "wb").write(sidecar(ir))
        for f in ("xlsx", "docx", "pdf"):
            open(os.path.join(layouts["ir+office"], f"{n}.{f}"), "wb").write(txt[f])
    sizes = {}
    for k, d in layouts.items():
        input_bytes = sum(os.path.getsize(os.path.join(d, f)) for f in os.listdir(d))
        stored, fails = ingest(d, os.path.join(work, "store-" + k))
        sizes[k] = stored
        out.write(json.dumps(dict(layout=k, input=input_bytes, stored=stored,
                                  fsck_failures=fails)) + "\n")
    store = os.path.join(work, "store-ir")
    exact = dict(csv=0, json=0, md=0)
    render_ms, read_ms = [], []
    for n in names:
        t = time.perf_counter()
        csv_b = cat(store, f"main:{n}.csv")
        side = cat(store, f"main:{n}.table.json")
        read_ms.append((time.perf_counter() - t) * 1e3)
        t = time.perf_counter()
        ir = from_sidecar(csv_b.decode(), side)
        js, md = ir.json().encode(), ir.md().encode()
        render_ms.append((time.perf_counter() - t) * 1e3)
        exact["csv"] += ir.csv().encode() == open(os.path.join(FIX, n + ".csv"), "rb").read()
        exact["json"] += js == open(os.path.join(FIX, n + ".json"), "rb").read()
        exact["md"] += md == open(os.path.join(FIX, n + ".md"), "rb").read()
    med = lambda v: round(sorted(v)[len(v) // 2], 3)
    s = dict(summary=True, tables=len(names), exact=exact, stored=sizes,
             read_two_files_ms=med(read_ms), render_json_md_ms=med(render_ms))
    out.write(json.dumps(s) + "\n")
    print(json.dumps(s))
    # Scaled (synthetic, as E161): render on read against reading the file.
    base = TableIR.lift(open(os.path.join(FIX, "s-local-retail-v01.csv")).read(),
                        open(os.path.join(FIX, "s-local-retail-v01.md")).read())
    for rows in (10_000, 100_000):
        big = TableIR(["Row"] + base.header,
                      [[str(i)] + base.rows[i % len(base.rows)] for i in range(rows)],
                      base.title, base.source,
                      [[True] + base.numeric[i % len(base.rows)] for i in range(rows)])
        d = os.path.join(work, f"scaled-{rows}")
        os.makedirs(d)
        open(os.path.join(d, "t.csv"), "w").write(big.csv())
        open(os.path.join(d, "t.json"), "w").write(big.json())
        open(os.path.join(d, "t.md"), "w").write(big.md())
        st = os.path.join(work, f"store-scaled-{rows}")
        ingest(d, st)
        row = dict(rows=rows)
        for f in ("json", "md"):
            t = time.perf_counter()
            cat(st, f"main:t.{f}")
            row[f + "_read_ms"] = round((time.perf_counter() - t) * 1e3, 1)
        t = time.perf_counter()
        ir = TableIR.lift(cat(st, "main:t.csv").decode(), big.md())
        js, md = ir.json(), ir.md()
        row["render_both_ms"] = round((time.perf_counter() - t) * 1e3, 1)
        row["exact"] = js == big.json() and md == big.md()
        out.write(json.dumps(row) + "\n")
        print(json.dumps(row))
    shutil.rmtree(work)


if __name__ == "__main__":
    main(sys.argv[1])
