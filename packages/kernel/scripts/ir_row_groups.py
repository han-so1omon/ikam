"""E220: a large table cut into row groups, each compressed on its own.

python3 -I packages/kernel/scripts/ir_row_groups.py tests/fixtures/san-diego-business-registry/*.csv

Prototype of the E213 layout (rows sorted by a key column kept in file
order, columns one after another, outer quotes of always-quoted columns
left out) applied to groups of rows, so a read of some rows decodes only
their group. Prints, against xz -9 over the files: the whole table, groups
compressed independently, and groups compressed after group 0 (as a
keyframe or shared dictionary would; estimated as xz(g0 + g) - xz(g0)),
with the time to decode one group plus group 0.
"""

import lzma
import sys
import time


def xz(b):
    return len(lzma.compress(b, preset=9))


def fields(line):
    out, start, quoted = [], 0, False
    for i, c in enumerate(line):
        if c == 34:
            quoted = not quoted
        elif c == 44 and not quoted:
            out.append(line[start:i])
            start = i + 1
    out.append(line[start:])
    return out


def is_quoted(f):
    return len(f) >= 2 and f[:1] == b'"' and f[-1:] == b'"'


def layout(rows, key):
    keys = [r[key] if key < len(r) else b"" for r in rows]
    order = sorted(range(len(rows)), key=lambda i: keys[i])
    rows_sorted = [rows[i] for i in order]
    out = []
    for k in range(max(len(r) for r in rows)):
        col = [r[k] for r in rows_sorted if len(r) > k]
        if all(is_quoted(f) for f in col):
            col = [f[1:-1] for f in col]
        out += col
    out += keys
    out += [str(len(r)).encode() for r in rows]
    return b"\n".join(out) + b"\n"


def main(paths):
    datas = [open(p, "rb").read() for p in paths]
    base = xz(b"".join(datas))
    key = fields(datas[0].split(b"\n")[0]).index(b"NAICS")
    tables = []
    for d in datas:
        lines = d.split(b"\n")
        if lines[-1] == b"":
            lines = lines[:-1]
        tables.append([fields(line) for line in lines[1:]])
    print("rows per file:", [len(t) for t in tables])
    print(f"whole tables: {xz(b''.join(layout(t, key) for t in tables)) / base:.3f}")
    for n in (30000, 10000, 3000, 1000):
        independent = keyed = groups = 0
        read = 0.0
        for t in tables:
            gs = [layout(t[i : i + n], key) for i in range(0, len(t), n)]
            first = xz(gs[0])
            independent += first
            keyed += first
            for g in gs[1:]:
                independent += xz(g)
                keyed += xz(gs[0] + g) - first
            groups += len(gs)
            z = lzma.compress(gs[0] + (gs[1] if len(gs) > 1 else b""), preset=9)
            start = time.time()
            lzma.decompress(z)
            read = max(read, time.time() - start)
        print(
            f"{n} rows ({groups} groups): independent {independent / base:.3f}, "
            f"after group 0 {keyed / base:.3f}, read one group {read * 1e3:.0f} ms"
        )


if __name__ == "__main__":
    main(sys.argv[1:])
