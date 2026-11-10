"""E184: aggregates over the 24 workbooks kept fresh from the computed-table
IR (E180) when an input cell changes.

Aggregates, from what each format shows: per workbook and in all, the total
of every dollar cell, the count of percent cells, and per column header the
total of its dollar cells.

Per input edit (E180's 22): the truth is the edited workbook recalculated
and exported by LibreOffice, with the other 23 workbooks' committed csv.
Policies:
- `full`: parse all 24 csv renders and aggregate again;
- `ir`: re-evaluate only the edited workbook's IR and apply the difference
  of its aggregates (the others are unchanged);
- `lazy`: leave the aggregates as they were; measured for how far off they
  are (the corpus dollar total, and columns whose total is wrong).

Run from packages/kernel: python3 scripts/table_aggregates.py OUT.jsonl
"""

import csv
import io
import json
import os
import shutil
import sys
import tempfile
import time
from collections import Counter

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from formula_ir import Sheet, truth  # noqa: E402
from table_ir import FIX  # noqa: E402


def dollars(t):
    t = t.strip()
    neg = t.startswith("-")
    t = t.lstrip("-")
    if not t.startswith("$"):
        return None
    try:
        v = int(t[1:].replace(",", ""))
    except ValueError:
        return None
    return -v if neg else v


def agg_of_grid(grid):
    """grid: rows of cell text (None for empty), header first."""
    a = Counter()
    header = grid[0]
    for row in grid[1:]:
        for h, x in zip(header, row):
            if x is None:
                continue
            d = dollars(x)
            if d is not None:
                a["$total"] += d
                a["col:" + (h or "")] += d
            elif x.endswith("%"):
                a["%count"] += 1
    return a


def agg_of_csv(text):
    rows = [r for r in csv.reader(io.StringIO(text))]
    return agg_of_grid([[c if c != "" else None for c in r] for r in rows])


def agg_of_ir(sh):
    return agg_of_grid([[x[0] if x else None for x in r] for r in sh.table()])


def total(per):
    t = Counter()
    for a in per.values():
        t.update(a)
    return t


def main(out_path):
    out = open(out_path, "w")
    work = tempfile.mkdtemp()
    names = sorted(n[:-5] for n in os.listdir(FIX) if n.endswith(".xlsx"))
    csvs = {n: open(os.path.join(FIX, n + ".csv"), encoding="utf-8", newline="").read()
            for n in names}
    sheets = {n: Sheet(os.path.join(FIX, n + ".xlsx"),
                       open(os.path.join(FIX, n + ".md"), encoding="utf-8").read()) for n in names}
    per = {n: agg_of_csv(csvs[n]) for n in names}
    assert all(agg_of_ir(sheets[n]) == per[n] for n in names)
    base = total(per)
    stats = dict(edits=0, exact=0, lazy_wrong=0)
    ms_full, ms_ir, read_full, err = [], [], [], []
    for n in names:
        sh = sheets[n]
        for at in [a for a, x in sh.cells.items() if x[0] == "f"]:
            inp = next((i for i in sh.inputs(at) if i in sh.cells and sh.cells[i][0] == "v"
                        and isinstance(sh.cells[i][1], (int, float))), None)
            if inp is None:
                continue
            old = sh.cells[inp]
            new_v = old[1] * 2 if old[1] else 7
            lo_csv, _ = truth(os.path.join(FIX, n + ".xlsx"), inp, new_v, work)
            want = total({**per, n: agg_of_csv(lo_csv)})
            # full: parse every render.
            texts = {**csvs, n: lo_csv}
            t = time.perf_counter()
            got_full = total({k: agg_of_csv(v) for k, v in texts.items()})
            ms_full.append((time.perf_counter() - t) * 1e3)
            read_full.append(sum(len(v.encode()) for v in texts.values()))
            # ir: re-evaluate one sheet, apply the difference.
            t = time.perf_counter()
            sh.cells[inp] = ("v", new_v, old[2])
            new_a = agg_of_ir(sh)
            got = Counter(base)
            got.subtract(per[n])
            got.update(new_a)
            got = Counter({k: v for k, v in got.items() if v or k in want})
            ms_ir.append((time.perf_counter() - t) * 1e3)
            sh.cells[inp] = old
            exact = got == want and got_full == want
            wrong_cols = sum(1 for k in set(want) | set(base) if want.get(k, 0) != base.get(k, 0))
            stats["edits"] += 1
            stats["exact"] += exact
            stats["lazy_wrong"] += wrong_cols > 0
            err.append(abs(want["$total"] - base["$total"]))
            out.write(json.dumps(dict(table=n, input=inp, exact=exact,
                                      lazy_wrong_aggregates=wrong_cols,
                                      lazy_dollar_error=want["$total"] - base["$total"])) + "\n")
    med = lambda v: round(sorted(v)[len(v) // 2], 3)  # noqa: E731
    s = dict(summary=True, workbooks=len(names), **stats, full_ms=med(ms_full),
             full_read=med(read_full), ir_ms=med(ms_ir),
             lazy_dollar_error_median=med(err), corpus_dollar_total=base["$total"])
    out.write(json.dumps(s) + "\n")
    print(json.dumps(s))
    shutil.rmtree(work)


if __name__ == "__main__":
    main(sys.argv[1])
