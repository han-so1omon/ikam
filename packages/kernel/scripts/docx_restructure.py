"""E169: E163's docx splice extended to edits that change the document's
structure: blocks inserted or deleted, new headings and new lists.

E163 replaced one block with one of the same shape. Here a range of md
blocks is replaced by any md text, and what pandoc numbers across the whole
document is renumbered after the splice:

1. at ingest, each md block's items in the old document.xml are recorded
   (each block rendered alone; this is kept like E163's block renders, and
   not timed);
2. the bookmarks are dropped from the body, the old blocks' items replaced
   by the new text's items (rendered alone), with their lists given fresh
   ids;
3. paragraph context styles (FirstParagraph or BodyText) are set for the
   first new item and the first item after the splice, from what follows
   what in the old document;
4. section bookmarks are rebuilt from the headings: a start before each
   heading, an end after the last item of its section (until a heading of
   the same or a higher level), ids 20 up in the order the ends appear,
   names kept from the old document or, for a new heading, taken from its
   render alone (made unique as pandoc does, with -1, -2);
5. lists are renumbered 1001 up in order of first use, and numbering.xml's
   num entries rebuilt in that order from the old document's and the new
   text's entries;
6. the package is rewritten with both parts replaced (E163's zip writer).

Checked per edit against a full pandoc re-render of the edited md, the
whole .docx byte for byte.

Edits per text (24 texts): a paragraph inserted, a paragraph deleted, a
heading with a paragraph inserted, a two-item list inserted, a heading
deleted, a paragraph between two lists deleted (markdown then joins
the lists), and E163's one-word edit (to check nothing regressed).

Run from packages/kernel: python3 scripts/docx_restructure.py OUT.jsonl
"""

import io
import json
import os
import re
import sys
import time
import zipfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from docx_splice import BOOK, NUM, STYLE, body, docx_xml, items, norm, rezip  # noqa: E402
from render_locality import FIX, blocks, edit_block, pandoc  # noqa: E402

HEAD = re.compile(rb'<w:pStyle w:val="Heading(\d)" />')
NAME = re.compile(rb'w:name="([^"]*)"')
NUMENTRY = re.compile(rb'<w:num w:numId="(\d+)">.*?</w:num>', re.S)


def parts(docx):
    z = zipfile.ZipFile(io.BytesIO(docx))
    return z.read("word/document.xml"), z.read("word/numbering.xml")


def content(xml):
    """Body items with bookmarks dropped, and the bookmark names by the
    index of the heading item they wrap."""
    a, z = body(xml)
    x = xml[a:z]
    out, names, pending = [], {}, None
    for s, e in items(x):
        it = x[s:e]
        if BOOK.match(it):
            if it.startswith(b"<w:bookmarkStart"):
                pending = NAME.search(it).group(1)
            continue
        if pending is not None:
            names[len(out)] = pending
            pending = None
        out.append(it)
    return out, names


def kind(it):
    if it is None:
        return b"none"
    if it.startswith(b"<w:tbl"):
        return b"table"
    if HEAD.search(it):
        return b"heading"
    if NUM.search(it):
        return b"list"
    m = STYLE.search(it)
    return m.group(1) if m else b"other"


def num_entries(numbering):
    return {m.group(1): m.group(0) for m in NUMENTRY.finditer(numbering)}


class Doc:
    """An old document, as kept at ingest."""

    def __init__(self, md):
        self.md = md
        self.bs = blocks(md)
        self.docx = pandoc(md, "docx", False)[0]
        self.xml, self.numbering = parts(self.docx)
        self.items, self.names = content(self.xml)
        # Each block's item range, by its render alone. Blocks markdown
        # joins in the document (a list continued past a blank line) are
        # kept as one group, rendered together.
        self.spans, self.group, k, g = [], [], 0, 0
        while g < len(self.bs):
            for e in range(g + 1, len(self.bs) + 1):
                alone = content(docx_xml(pandoc(b"".join(self.bs[g:e]), "docx", False)[0]))[0]
                n = len(alone)
                if [norm(i) for i in self.items[k:k + n]] == [norm(i) for i in alone]:
                    break
            else:
                raise ValueError("block items not found")
            for _ in range(g, e):
                self.spans.append((k, k + n))
                self.group.append((g, e))
            k, g = k + n, e
        if k != len(self.items):
            raise ValueError("items left over")
        # What style a paragraph takes after each kind of item.
        self.after = {}
        for prev, it in zip([None] + self.items[:-1], self.items):
            m = STYLE.search(it)
            if m:
                self.after.setdefault(kind(prev), m.group(1))


def alone(md):
    if not md:
        return [], {}, {}
    nxml, nnum = parts(pandoc(md, "docx", False)[0])
    its, names = content(nxml)
    return its, names, num_entries(nnum)


def render(doc, i, j, new_md):
    """doc's md with blocks i..j replaced by new_md, rendered by splicing.

    Where the new text would touch a list on either side, markdown may join
    the two lists, so the neighbouring block is taken into the splice and
    rendered with the new text."""
    widened = 0
    if i < len(doc.bs) and doc.group[i][0] < i:
        g = doc.group[i][0]
        new_md, i = b"".join(doc.bs[g:i]) + new_md, g
    if j > i and doc.group[j - 1][1] > j:
        e = doc.group[j - 1][1]
        new_md, j = new_md + b"".join(doc.bs[j:e]), e
    while True:
        new_items, new_names, new_nums = alone(new_md)
        a = doc.spans[i][0] if i < len(doc.spans) else len(doc.items)
        z = doc.spans[j - 1][1] if j > i else a
        left = doc.items[a - 1] if a else None
        right = doc.items[z] if z < len(doc.items) else None
        first = new_items[0] if new_items else right
        last = new_items[-1] if new_items else left
        if kind(left) == b"list" and kind(first) == b"list" and i > 0:
            g = doc.group[i - 1][0]
            new_md = b"".join(doc.bs[g:i]) + new_md
            i = g
        elif kind(last) == b"list" and kind(right) == b"list" and j < len(doc.bs):
            e = doc.group[j][1]
            new_md = new_md + b"".join(doc.bs[j:e])
            j = e
        else:
            break
        widened += 1
    # Fresh list ids for the new items, kept apart from the old ones.
    src = num_entries(doc.numbering)
    fresh = {}
    for n in new_nums:
        fresh[n] = b"9" + n
        src[b"9" + n] = new_nums[n]
    new_items = [NUM.sub(lambda m: m.group(1) + fresh.get(m.group(2), m.group(2)) + m.group(3),
                         it) for it in new_items]
    out = doc.items[:a] + new_items + doc.items[z:]
    names = {k: v for k, v in doc.names.items() if k < a}
    names.update({a + k: v for k, v in new_names.items()})
    names.update({k - (z - a) + len(new_items): v for k, v in doc.names.items() if k >= z})
    # Context styles at the two seams.
    for k in (a, a + len(new_items)):
        if k < len(out):
            m = STYLE.search(out[k])
            want = doc.after.get(kind(out[k - 1] if k else None))
            if m and want and m.group(1) != want:
                out[k] = out[k].replace(m.group(0), m.group(0).replace(m.group(1), want), 1)
    # New heading names made unique against the rest.
    taken = {v for k, v in names.items() if not (a <= k < a + len(new_items))}
    for k in range(a, a + len(new_items)):
        if k in names and names[k] in taken:
            base, n = names[k], 1
            while base + b"-%d" % n in taken:
                n += 1
            names[k] = base + b"-%d" % n
        if k in names:
            taken.add(names[k])
    # Sections: each heading runs to the next of its level or higher.
    level = {k: int(HEAD.search(it).group(1)) for k, it in enumerate(out) if HEAD.search(it)}
    ends = {}
    for k, lv in level.items():
        e = next((m for m in sorted(level) if m > k and level[m] <= lv), len(out))
        ends.setdefault(e, []).append(k)
    order = []  # headings by where their bookmark ends (inner ends first)
    for e in sorted(ends):
        order += sorted(ends[e], reverse=True)
    bid = {k: 20 + r for r, k in enumerate(order)}
    body_x = bytearray()
    for k, it in enumerate(out + [None]):
        for h in sorted(ends.get(k, []), reverse=True):
            body_x += b'<w:bookmarkEnd w:id="%d" />' % bid[h]
        if it is None:
            break
        if k in level:
            body_x += b'<w:bookmarkStart w:id="%d" w:name="%s" />' % (bid[k], names[k])
        body_x += it
    # Lists renumbered in order of first use.
    seen = {}
    for m in NUM.finditer(bytes(body_x)):
        seen.setdefault(m.group(2), b"%d" % (1001 + len(seen)))
    body_b = NUM.sub(lambda m: m.group(1) + seen[m.group(2)] + m.group(3), bytes(body_x))
    ba, bz = body(doc.xml)
    xml = doc.xml[:ba] + body_b + doc.xml[bz:]
    # num entries below 1001 (pandoc's 1000, used by no list) stay as they are.
    first = doc.numbering.index(b'<w:num w:numId="1001"') if b'w:numId="1001"' in \
        doc.numbering else doc.numbering.rindex(b"</w:numbering>")
    tail = doc.numbering.rindex(b"</w:numbering>")
    nums = b"".join(re.sub(rb'w:numId="\d+"', b'w:numId="' + new + b'"', src[old], count=1)
                    for old, new in seen.items())
    numbering = doc.numbering[:first] + nums + doc.numbering[tail:]
    return rezip(doc.docx, {"word/document.xml": xml, "word/numbering.xml": numbering})


def pick(bs, want, start):
    """First block index from start (wrapping) whose kind fits."""
    for d in range(len(bs)):
        k = (start + d) % len(bs)
        if want(k):
            return k
    return None


def is_list(b):
    return re.match(rb"\s*([-*+]|\d+[.)])\s", b) is not None


def is_head(b):
    return b.startswith(b"#")


def edits(bs):
    n = len(bs)
    out = []
    plain = lambda k: not is_list(bs[k]) and not is_head(bs[k])  # noqa: E731
    k = pick(bs, lambda k: k > 0 and plain(k), n // 2)
    if k is not None:
        out.append(("insert-paragraph", k, k, b"Added note: this paragraph was inserted.\n\n"))
    k = pick(bs, lambda k: plain(k) and k > 0, n // 3)
    if k is not None:
        out.append(("delete-paragraph", k, k + 1, b""))
    k = pick(bs, lambda k: k > 0 and plain(k), 2 * n // 3)
    if k is not None:
        out.append(("insert-heading", k, k,
                    b"## Added section\n\nAdded paragraph under the new heading.\n\n"))
    k = pick(bs, lambda k: k > 0 and not is_list(bs[k]) and not is_list(bs[k - 1]), n // 4)
    if k is not None:
        out.append(("insert-list", k, k, b"- added item one\n- added item two\n\n"))
    k = pick(bs, lambda k: bs[k].startswith(b"## ") and k > 0, n // 2)
    if k is not None:
        out.append(("delete-heading", k, k + 1, b""))
    # A paragraph between two lists deleted, so markdown joins the lists.
    ends_list = lambda b: is_list(b.rstrip(b"\n").split(b"\n")[-1].lstrip())  # noqa: E731
    k = pick(bs, lambda k: 0 < k < n - 1 and plain(k) and ends_list(bs[k - 1])
             and is_list(bs[k + 1]), n // 2)
    if k is not None:
        out.append(("delete-between-lists", k, k + 1, b""))
    k = n // 2
    nb = edit_block(bs[k])
    if nb is not None:
        out.append(("word", k, k + 1, nb))
    return out


def main(out_path):
    names = sorted(n[:-3] for n in os.listdir(FIX) if n.endswith(".md"))
    out = open(out_path, "w")
    tot, ms_full, ms_splice = {}, [], []
    for name in names:
        md = open(os.path.join(FIX, name + ".md"), "rb").read()
        doc = Doc(md)
        for op, i, j, new in edits(doc.bs):
            md2 = b"".join(doc.bs[:i]) + new + b"".join(doc.bs[j:])
            full, tf = pandoc(md2, "docx", False)
            t = time.perf_counter()
            try:
                got, err = render(doc, i, j, new), None
            except Exception as e:  # noqa: BLE001
                got, err = None, repr(e)
            ts = time.perf_counter() - t
            exact = got == full
            row = dict(text=name, op=op, block=i, exact=exact, full_ms=round(tf * 1e3, 1),
                       splice_ms=round(ts * 1e3, 1))
            if err:
                row["error"] = err
            elif not exact:
                for part in ("word/document.xml", "word/numbering.xml"):
                    g = zipfile.ZipFile(io.BytesIO(got)).read(part)
                    w = zipfile.ZipFile(io.BytesIO(full)).read(part)
                    if g != w:
                        d = next((q for q in range(min(len(g), len(w))) if g[q] != w[q]),
                                 min(len(g), len(w)))
                        row["diff"] = [part, g[max(0, d - 80):d + 80].decode(errors="replace"),
                                       w[max(0, d - 80):d + 80].decode(errors="replace")]
                        break
            out.write(json.dumps(row) + "\n")
            t = tot.setdefault(op, [0, 0])
            t[0] += 1
            t[1] += exact
            ms_full.append(tf * 1e3)
            ms_splice.append(ts * 1e3)
    # No text has a paragraph between two lists, so one is made: two lists
    # and a paragraph inserted mid-text (the left list may join the list
    # before it, which ingest keeps as one group), then the paragraph
    # deleted, which joins the two lists.
    t = tot.setdefault("delete-between-lists (synthetic)", [0, 0])
    for name in names:
        bs = blocks(open(os.path.join(FIX, name + ".md"), "rb").read())
        k = len(bs) // 2
        md = b"".join(bs[:k]) + b"- left list item\n\nBetween paragraph.\n\n- right list item\n\n" \
            + b"".join(bs[k:])
        doc = Doc(md)
        i = next(x for x, b in enumerate(doc.bs) if b.startswith(b"Between paragraph"))
        md2 = b"".join(doc.bs[:i]) + b"".join(doc.bs[i + 1:])
        t[0] += 1
        t[1] += render(doc, i, i + 1, b"") == pandoc(md2, "docx", False)[0]
    med = lambda v: round(sorted(v)[len(v) // 2], 1)  # noqa: E731
    s = dict(summary=True, exact={k: f"{v[1]}/{v[0]}" for k, v in tot.items()},
             full_ms=med(ms_full), splice_ms=med(ms_splice))
    out.write(json.dumps(s) + "\n")
    print(json.dumps(s))


if __name__ == "__main__":
    main(sys.argv[1])
