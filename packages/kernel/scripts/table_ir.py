"""E161: a table intermediate representation (the Python-era TableIR /
StructuredDataIR, recovered from b1ce98f^) as the one place a cell edit is
made, with every format rendered from it.

For the 24 workbooks in fixtures/tables:

- lift: the csv's records into a TableIR (header, rows of cell text) plus
  residuals that the formats carry and the table does not (md: title and
  source line). Lifting only reads the csv; json and md are rendered.
- render: csv, json and md from the IR by the rules of render.py, checked
  byte for byte against the committed files (the drift spec is byte
  identity, as the Python Verifier had it);
- store: the kernel's bytes for the three formats put as files, against
  the IR alone (the csv plus residuals), with the others rendered on read;
- one-cell edit (the middle row's last numeric-looking cell, or its last
  cell): re-render csv, json and md from the edited IR; check each new file
  differs from the old in one contiguous range (the edit stays local); the
  bytes the kernel writes for each new file; render times;
- regenerate endpoints: docx (pandoc from the new md) and pdf
  (LibreOffice from the docx), timed, and the kernel's bytes for them.
  These are not byte-identical to the committed ones (tool timestamps), so
  they are regenerated, not spliced.

A scaled run (synthetic: one workbook's rows repeated to 10,000 with a row
number) shows how render and write costs grow with table size.

Run from packages/kernel after `cargo build --release`:
    python3 scripts/table_ir.py OUT.jsonl
"""

import csv
import io
import json
import os
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
KERNEL = os.path.dirname(HERE)
FIX = os.path.join(KERNEL, "fixtures", "tables")
IKAM = os.path.join(KERNEL, "target", "release", "ikam")
ENV = dict(os.environ, SOURCE_DATE_EPOCH="1767225600")


def number(text):
    for kind in (int, float):
        try:
            return kind(text)
        except ValueError:
            pass
    return text


def cell(text):
    return text.replace("|", "\\|")


def typed_fields(line):
    """A csv line's fields and whether each was written unquoted (the
    spreadsheet typed it as a number: LibreOffice quotes text cells only)."""
    out, i, n = [], 0, len(line)
    while i <= n:
        if i < n and line[i] == '"':
            j, buf = i + 1, []
            while True:
                if line[j] == '"' and j + 1 < n and line[j + 1] == '"':
                    buf.append('"')
                    j += 2
                elif line[j] == '"':
                    break
                else:
                    buf.append(line[j])
                    j += 1
            out.append(("".join(buf), False))
            i = j + 2
        else:
            j = line.find(",", i)
            j = n if j < 0 else j
            out.append((line[i:j], line[i:j] != ""))
            i = j + 1
    return out


class TableIR:
    """Header, rows of cell text, and per cell whether the spreadsheet typed
    it as a number (a cell type the csv's quoting carries)."""

    def __init__(self, header, rows, title, source, numeric=None):
        self.header, self.rows, self.title, self.source = header, rows, title, source
        self.numeric = numeric or [[False] * len(r) for r in rows]
        self.blank = []

    @staticmethod
    def lift(csv_text, md_text, typed=True):
        all_lines = csv_text.split("\n")[:-1]
        lines = [ln for ln in all_lines if ln.strip(",")]
        # A layout residual: where the csv has empty rows, which the table
        # (and json and md) leave out.
        blank = [(i, ln) for i, ln in enumerate(all_lines) if not ln.strip(",")]
        if typed:
            parsed = [typed_fields(ln) for ln in lines]
            rows = [[v for v, _ in r] for r in parsed]
            numeric = [[t for _, t in r] for r in parsed[1:]]
        else:
            rows = [r for r in csv.reader(io.StringIO(csv_text)) if any(r)]
            numeric = None
        md = md_text.split("\n")
        ir = TableIR(rows[0], rows[1:], md[0][2:], md[2], numeric)
        ir.blank = blank if typed else []
        return ir

    def residual_bytes(self):
        return len(self.title.encode()) + len(self.source.encode()) + sum(
            4 + len(ln) for _, ln in self.blank)

    def csv(self):
        q = lambda v: '"' + v.replace('"', '""') + '"'
        lines = [",".join(q(h) for h in self.header)]
        for r, t in zip(self.rows, self.numeric):
            lines.append(",".join(v if n else (q(v) if v else "") for v, n in zip(r, t)))
        for i, ln in self.blank:
            lines.insert(i, ln)
        return "\n".join(lines) + "\n"

    def json(self):
        objects = [{k: number(v) for k, v in zip(self.header, r) if v != ""} for r in self.rows]
        return json.dumps(objects, indent=2, ensure_ascii=False) + "\n"

    def md(self):
        md = [f"# {self.title}", "", self.source, ""]
        md.append("| " + " | ".join(cell(h) for h in self.header) + " |")
        md.append("|" + "---|" * len(self.header))
        md += ["| " + " | ".join(cell(v) for v in r) + " |" for r in self.rows]
        return "\n".join(md) + "\n"


def one_range(a, b):
    """Whether b differs from a in one contiguous range."""
    i = 0
    while i < min(len(a), len(b)) and a[i] == b[i]:
        i += 1
    j = 0
    while j < min(len(a), len(b)) - i and a[-1 - j] == b[-1 - j]:
        j += 1
    return a[:i] + b[i:len(b) - j] + a[len(a) - j:] == b, len(b) - i - j


def store_size(d):
    return sum(os.path.getsize(os.path.join(r, f)) for r, _, fs in os.walk(d) for f in fs)


def snapshot(d):
    out = {}
    for r, _, fs in os.walk(d):
        for f in fs:
            p = os.path.join(r, f)
            st = os.stat(p)
            out[p] = (st.st_size, st.st_mtime_ns)
    return out


def put(store, data, suffix):
    """Bytes of store files the put created or rewrote."""
    with tempfile.NamedTemporaryFile(suffix=suffix, delete=False) as f:
        f.write(data)
    before = snapshot(store)
    subprocess.run([IKAM, "--store", store, "put", f.name], capture_output=True, check=True)
    os.unlink(f.name)
    return sum(sz for p, (sz, m) in snapshot(store).items() if before.get(p) != (sz, m))


def edit(ir):
    """Edit the middle row's last numeric-looking cell (or its last cell)."""
    r = len(ir.rows) // 2
    row = ir.rows[r]
    cols = [i for i, v in enumerate(row) if any(c.isdigit() for c in v)] or [len(row) - 1]
    c = cols[-1]
    new = row[c] + "1" if row[c] else "1"
    ir.rows[r] = row[:c] + [new] + row[c + 1:]
    return r, c


def timed(f):
    t = time.perf_counter()
    v = f()
    return v, (time.perf_counter() - t) * 1e3


def run_table(name, csv_text, md_text, json_text, store, work, out, regen=True, label=None):
    ir, lift_ms = timed(lambda: TableIR.lift(csv_text, md_text))
    exact = dict(csv=ir.csv() == csv_text, json=ir.json() == json_text, md=ir.md() == md_text,
                 csv_untyped=TableIR.lift(csv_text, md_text, typed=False).csv() == csv_text)
    files = dict(csv=csv_text, json=json_text, md=md_text)
    kernel_three = sum(put(store, files[f].encode(), "." + f) for f in files)
    ir_alone = len(csv_text.encode()) + ir.residual_bytes()
    r, c = edit(ir)
    row = dict(table=label or name, rows=len(ir.rows), cols=len(ir.header),
               exact=exact, lift_ms=round(lift_ms, 3), kernel_bytes_three=kernel_three,
               ir_bytes=ir_alone, edit_row=r, edit_col=c)
    for f in ("csv", "json", "md"):
        new, ms = timed(getattr(ir, f))
        local, changed = one_range(files[f], new)
        row[f] = dict(render_ms=round(ms, 3), local=local, changed_bytes=changed,
                      written=put(store, new.encode(), "." + f))
    if regen:
        mdp = os.path.join(work, "t.md")
        with open(mdp, "w") as fh:
            fh.write(ir.md())
        docx, ms = timed(lambda: subprocess.run(
            ["pandoc", mdp, "-o", "-", "-t", "docx"], capture_output=True, env=ENV,
            check=True).stdout)
        dp = os.path.join(work, "t.docx")
        with open(dp, "wb") as fh:
            fh.write(docx)
        _, pms = timed(lambda: subprocess.run(
            ["soffice", "--headless", "--convert-to", "pdf", "--outdir", work, dp],
            capture_output=True, env=ENV, check=True))
        pdf = open(os.path.join(work, "t.pdf"), "rb").read()
        row["docx"] = dict(render_ms=round(ms, 1), written=put(store, docx, ".docx"))
        row["pdf"] = dict(render_ms=round(pms, 1), written=put(store, pdf, ".pdf"))
    out.write(json.dumps(row) + "\n")
    out.flush()
    return row


def main(out_path):
    work = tempfile.mkdtemp()
    out = open(out_path, "w")
    names = sorted(n[:-4] for n in os.listdir(FIX) if n.endswith(".csv"))
    rows = []
    for name in names:
        store = os.path.join(work, "store-" + name)
        for f in ("docx", "pdf"):
            subprocess.run([IKAM, "--store", store, "put", os.path.join(FIX, f"{name}.{f}")],
                           capture_output=True, check=True)
        txt = {f: open(os.path.join(FIX, f"{name}.{f}"), encoding="utf-8", newline="").read()
               for f in ("csv", "md", "json")}
        rows.append(run_table(name, txt["csv"], txt["md"], txt["json"], store, work, out))
    s = dict(summary=True, tables=len(rows))
    for f in ("csv", "json", "md"):
        s[f + "_exact"] = sum(r["exact"][f] for r in rows)
        s[f + "_local"] = sum(r[f]["local"] for r in rows)
        s[f + "_written"] = sum(r[f]["written"] for r in rows)
    for f in ("docx", "pdf"):
        s[f + "_written"] = sum(r[f]["written"] for r in rows)
        s[f + "_ms_median"] = sorted(r[f]["render_ms"] for r in rows)[len(rows) // 2]
    s["csv_untyped_exact"] = sum(r["exact"]["csv_untyped"] for r in rows)
    s["kernel_bytes_three"] = sum(r["kernel_bytes_three"] for r in rows)
    s["ir_bytes"] = sum(r["ir_bytes"] for r in rows)
    out.write(json.dumps(s) + "\n")
    print(json.dumps(s))
    # Scaled (synthetic): one table's rows repeated, numbered.
    name = "s-local-retail-v01"
    base = TableIR.lift(open(os.path.join(FIX, name + ".csv"), encoding="utf-8").read(),
                        open(os.path.join(FIX, name + ".md"), encoding="utf-8").read())
    for n in (100, 1000, 10000):
        big = TableIR(["Row"] + base.header,
                      [[str(i)] + base.rows[i % len(base.rows)] for i in range(n)],
                      base.title, base.source,
                      [[True] + base.numeric[i % len(base.rows)] for i in range(n)])
        store = os.path.join(work, f"store-scaled-{n}")
        r = run_table(name, big.csv(), big.md(), big.json(), store, work, out,
                      regen=False, label=f"scaled-{n}")
        print(json.dumps({k: r[k] for k in ("table", "rows", "csv", "json", "md")}))


if __name__ == "__main__":
    main(sys.argv[1])
