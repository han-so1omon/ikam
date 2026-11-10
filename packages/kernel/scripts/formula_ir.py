"""E180: a computed-table IR (the Python version's StructuredDataIR with
ExpressionIR): cell values, number formats and formulas as shapes, so an
edit to an input cell reaches every cell computed from it, in every
rendered format.

E161's TableIR is lifted from the csv, so it holds what LibreOffice showed,
and a formula's result is only text. Here the IR is lifted from the
workbook's first sheet (fixtures/tables, 24 workbooks, 9 with formulas):

- per cell: its value (number or text), its number format, or its formula;
- a formula is an OpShape (its AST with each reference made relative to the
  cell, so B2*12 in C2 and B3*12 in C3 are one shape) and an OpInstance
  (the cell it sits in);
- residuals: the md's title and source line (as in E161).

A deterministic renderer evaluates the formulas (+ - * /, comparisons,
IF, AND, SUM; a cycle is LibreOffice's Err:522), shows each value by its
number format (General, $#,##0, 0%, 0.0%; a General formula cell takes the
format of the first cell it refers to, as LibreOffice does), and writes csv
(LibreOffice's quoting: text quoted, a number quoted when its text holds a
comma), json and md (E161's renderers).

Checked:
1. the IR's renders of each workbook against the committed csv, json and md;
2. per formula, an edit to the first input cell it reads (the value
   doubled, or set to 7 when it is 0): the IR recomputes and renders, and the
   truth is the workbook edited with openpyxl, recalculated and exported by
   LibreOffice as fixtures/tables/render.py does, json and md made from that
   csv.

Run from packages/kernel: python3 scripts/formula_ir.py OUT.jsonl
"""

import io
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

import openpyxl
from openpyxl.utils import get_column_letter

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from table_ir import FIX, TableIR  # noqa: E402

TOK = re.compile(r'\s*(?:(\$?[A-Z]+\$?\d+(?::\$?[A-Z]+\$?\d+)?)|(\d+(?:\.\d+)?)|("(?:[^"]|"")*")'
                 r'|([A-Z]+)\(|(<=|>=|<>|[-+*/=<>(),&^]))')


def col_num(s):
    n = 0
    for ch in s:
        n = n * 26 + ord(ch) - 64
    return n


def ref(s):
    m = re.fullmatch(r"\$?([A-Z]+)\$?(\d+)", s)
    return int(m.group(2)), col_num(m.group(1))


def tokens(f):
    out, i = [], 0
    f = f.strip()
    while i < len(f):
        m = TOK.match(f, i)
        if not m or m.end() == i:
            raise ValueError("cannot read formula " + f)
        r, num, st, fn, op = m.groups()
        out.append(("ref", r) if r else ("num", float(num)) if num else
                   ("str", st[1:-1].replace('""', '"')) if st else ("fn", fn) if fn else ("op", op))
        i = m.end()
    return out


def parse(f, at):
    """AST of formula f (without '=') in cell at, with references relative
    to at, so the AST is the formula's shape."""
    ts, k = tokens(f), [0]

    def peek():
        return ts[k[0]] if k[0] < len(ts) else (None, None)

    def take():
        k[0] += 1
        return ts[k[0] - 1]

    def rel(s):
        r, c = ref(s)
        return (r - at[0], c - at[1])

    def atom():
        t, v = take()
        if t == "num":
            return ("num", v)
        if t == "str":
            return ("str", v)
        if t == "ref":
            if ":" in v:
                a, b = v.split(":")
                return ("range", rel(a), rel(b))
            return ("ref", rel(v))
        if t == "fn":
            args = []
            if peek() != ("op", ")"):
                args.append(compare())
                while peek() == ("op", ","):
                    take()
                    args.append(compare())
            assert take() == ("op", ")")
            return ("fn", v, tuple(args))
        if (t, v) == ("op", "("):
            e = compare()
            assert take() == ("op", ")")
            return e
        if (t, v) == ("op", "-"):
            return ("neg", atom())
        raise ValueError(f"unexpected {v} in {f}")

    def binary(sub, ops):
        def go():
            e = sub()
            while peek()[0] == "op" and peek()[1] in ops:
                op = take()[1]
                e = ("op", op, e, sub())
            return e
        return go

    term = binary(atom, ("*", "/"))
    arith = binary(term, ("+", "-"))
    compare = binary(arith, ("=", "<>", "<", ">", "<=", ">="))
    e = compare()
    assert k[0] == len(ts), f
    return e


def shape_key(ast):
    return json.dumps(ast)


class Sheet:
    """The IR: cells by (row, col): ('v', value, fmt) or ('f', shape, fmt)."""

    def __init__(self, path, md_text):
        ws = openpyxl.load_workbook(path).worksheets[0]
        self.rows, self.cols = ws.max_row, ws.max_column
        self.cells, self.shapes = {}, {}
        for row in ws.iter_rows():
            for c in row:
                if c.value is None:
                    continue
                at = (c.row, c.column)
                if isinstance(c.value, str) and c.value.startswith("="):
                    ast = parse(c.value[1:], at)
                    key = shape_key(ast)
                    self.shapes.setdefault(key, ast)
                    self.cells[at] = ("f", key, c.number_format)
                else:
                    self.cells[at] = ("v", c.value, c.number_format)
        md = md_text.split("\n")
        self.title, self.source = md[0][2:], md[2]

    def inputs(self, at):
        """Cells a formula cell reads (absolute), in order."""
        out = []

        def walk(e):
            if e[0] == "ref":
                out.append((at[0] + e[1][0], at[1] + e[1][1]))
            elif e[0] == "range":
                for r in range(at[0] + e[1][0], at[0] + e[2][0] + 1):
                    for c in range(at[1] + e[1][1], at[1] + e[2][1] + 1):
                        out.append((r, c))
            elif e[0] in ("op",):
                walk(e[2])
                walk(e[3])
            elif e[0] == "neg":
                walk(e[1])
            elif e[0] == "fn":
                for a in e[2]:
                    walk(a)
        walk(self.shapes[self.cells[at][1]])
        return out

    def evaluate(self):
        vals, state = {}, {}

        def get(at):
            c = self.cells.get(at)
            if c is None:
                return None
            if c[0] == "v":
                return c[1]
            if state.get(at) == "busy":
                raise Cycle()
            if at not in vals:
                state[at] = "busy"
                try:
                    vals[at] = ev(self.shapes[c[1]], at)
                except Cycle:
                    vals[at] = Err522
                state[at] = "done"
            return vals[at]

        def num(v):
            if v is Err522:
                raise Cycle()
            return 0 if v is None or v == "" else v

        def ev(e, at):
            t = e[0]
            if t == "num" or t == "str":
                return e[1]
            if t == "ref":
                return get((at[0] + e[1][0], at[1] + e[1][1]))
            if t == "neg":
                return -num(ev(e[1], at))
            if t == "op":
                a, b, op = ev(e[2], at), ev(e[3], at), e[1]
                if op in "+-*/":
                    a, b = num(a), num(b)
                    return {"+": a + b, "-": a - b, "*": a * b, "/": a / b}[op]
                return {"=": a == b, "<>": a != b, "<": a < b, ">": a > b,
                        "<=": a <= b, ">=": a >= b}[op]
            if t == "fn":
                name, args = e[1], e[2]
                if name == "IF":
                    return ev(args[1], at) if ev(args[0], at) else ev(args[2], at)
                if name == "AND":
                    return all(ev(a, at) for a in args)
                if name == "SUM":
                    s = 0
                    for a in args:
                        if a[0] == "range":
                            for r in range(at[0] + a[1][0], at[0] + a[2][0] + 1):
                                for c in range(at[1] + a[1][1], at[1] + a[2][1] + 1):
                                    v = get((r, c))
                                    if v is Err522:
                                        raise Cycle()
                                    if isinstance(v, (int, float)) and not isinstance(v, bool):
                                        s += v
                        else:
                            s += num(ev(a, at))
                    return s
            raise ValueError(f"no evaluator for {e}")

        for at in sorted(self.cells):
            if self.cells[at][0] == "f":
                get(at)
        return vals

    def fmt_of(self, at):
        c = self.cells[at]
        if c[0] == "f" and c[2] == "General":
            for i in self.inputs(at):
                if i in self.cells and self.cells[i][2] != "General":
                    return self.cells[i][2]
        return c[2]

    def table(self):
        vals = self.evaluate()
        grid = []
        for r in range(1, self.rows + 1):
            line = []
            for c in range(1, self.cols + 1):
                at = (r, c)
                if at not in self.cells:
                    line.append(None)
                    continue
                v = vals[at] if self.cells[at][0] == "f" else self.cells[at][1]
                line.append(show(v, self.fmt_of(at)))
            grid.append(line)
        return grid

    def csv(self):
        out = []
        for line in self.table():
            f = []
            for x in line:
                if x is None:
                    f.append("")
                else:
                    text, is_num = x
                    f.append('"' + text.replace('"', '""') + '"' if not is_num or "," in text
                             else text)
            out.append(",".join(f))
        return "\n".join(out) + "\n"

    def renders(self):
        csv_text = self.csv()
        md = f"# {self.title}\n\n{self.source}\n"
        ir = TableIR.lift(csv_text, md)
        return csv_text, ir.json(), ir.md()


class Cycle(Exception):
    pass


Err522 = object()


def general(v):
    if float(v).is_integer():
        return str(int(v))
    s = f"{v:.10g}"
    return s


def show(v, fmt):
    """(text as LibreOffice shows it, whether it is a number)."""
    if v is Err522:
        return "Err:522", False
    if isinstance(v, bool):
        return ("TRUE" if v else "FALSE"), True
    if isinstance(v, str):
        return v, False
    if fmt == "$#,##0":
        r = round_half_up(v, 0)
        return ("-" if r < 0 else "") + "$" + f"{abs(int(r)):,}", True
    if fmt in ("0%", "0.0%"):
        d = 0 if fmt == "0%" else 1
        return f"{round_half_up(v * 100, d):.{d}f}%", True
    return general(v), True


def round_half_up(v, d):
    from decimal import ROUND_HALF_UP, Decimal
    q = Decimal(1).scaleb(-d)
    return float(Decimal(repr(v)).quantize(q, rounding=ROUND_HALF_UP))


def truth(xlsx, at, value, work):
    """The workbook with cell at set to value, recalculated and exported by
    LibreOffice: (csv, json, md made from it as render.py does)."""
    wb = openpyxl.load_workbook(xlsx)
    wb.worksheets[0].cell(row=at[0], column=at[1]).value = value
    d = tempfile.mkdtemp(dir=work)
    p = os.path.join(d, "t.xlsx")
    wb.save(p)
    t = time.perf_counter()
    subprocess.run(["soffice", "--headless", "--convert-to",
                    "csv:Text - txt - csv (StarCalc):44,34,76,1", "--outdir", d, p],
                   check=True, capture_output=True)
    ms = (time.perf_counter() - t) * 1e3
    return open(os.path.join(d, "t.csv"), encoding="utf-8", newline="").read(), ms


def main(out_path):
    out = open(out_path, "w")
    work = tempfile.mkdtemp()
    names = sorted(n[:-5] for n in os.listdir(FIX) if n.endswith(".xlsx"))
    base = dict(csv=0, json=0, md=0)
    shapes = formulas = 0
    tot = dict(edits=0, csv=0, json=0, md=0, changed_cells=0)
    ir_ms, lo_ms = [], []
    for n in names:
        md_text = open(os.path.join(FIX, n + ".md"), encoding="utf-8").read()
        sh = Sheet(os.path.join(FIX, n + ".xlsx"), md_text)
        c, j, m = sh.renders()
        ok = dict(csv=c == open(os.path.join(FIX, n + ".csv"), encoding="utf-8", newline="").read(),
                  json=j == open(os.path.join(FIX, n + ".json"), encoding="utf-8").read(),
                  md=m == open(os.path.join(FIX, n + ".md"), encoding="utf-8").read())
        for k in base:
            base[k] += ok[k]
        fs = [at for at, x in sh.cells.items() if x[0] == "f"]
        formulas += len(fs)
        shapes += len({sh.cells[at][1] for at in fs})
        row = dict(table=n, formulas=len(fs), shapes=len({sh.cells[at][1] for at in fs}), **ok)
        if not ok["csv"]:
            row["got"] = c[:300]
        out.write(json.dumps(row) + "\n")
        for at in fs:
            inp = next((i for i in sh.inputs(at) if i in sh.cells and sh.cells[i][0] == "v"
                        and isinstance(sh.cells[i][1], (int, float))), None)
            if inp is None:
                continue
            old = sh.cells[inp]
            new_v = old[1] * 2 if old[1] else 7
            before = sh.table()
            t = time.perf_counter()
            sh.cells[inp] = ("v", new_v, old[2])
            c, j, m = sh.renders()
            ir_ms.append((time.perf_counter() - t) * 1e3)
            after = sh.table()
            sh.cells[inp] = old
            tc, ms = truth(os.path.join(FIX, n + ".xlsx"), inp, new_v, work)
            lo_ms.append(ms)
            tir = TableIR.lift(tc, md_text)
            ex = dict(csv=c == tc, json=j == tir.json(), md=m == tir.md())
            changed = sum(a != b for ra, rb in zip(before, after) for a, b in zip(ra, rb))
            tot["edits"] += 1
            tot["changed_cells"] += changed
            for k in ("csv", "json", "md"):
                tot[k] += ex[k]
            r = dict(table=n, formula=get_column_letter(at[1]) + str(at[0]),
                     input=get_column_letter(inp[1]) + str(inp[0]), new=new_v,
                     changed_cells=changed, **ex)
            if not ex["csv"]:
                r["got"], r["want"] = c[:300], tc[:300]
            out.write(json.dumps(r) + "\n")
    med = lambda v: round(sorted(v)[len(v) // 2], 2)  # noqa: E731
    s = dict(summary=True, tables=len(names), base_exact=base, formulas=formulas,
             shapes=shapes, **{"edit_" + k: v for k, v in tot.items()},
             ir_ms=med(ir_ms), libreoffice_ms=med(lo_ms))
    out.write(json.dumps(s) + "\n")
    print(json.dumps(s))
    shutil.rmtree(work)


if __name__ == "__main__":
    main(sys.argv[1])
