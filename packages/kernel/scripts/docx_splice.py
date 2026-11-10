"""E163: a docx renderer that splices one edited paragraph block into the
old document instead of re-rendering it.

E158 found pandoc's docx is not local: it numbers bookmarks (w:id, in
section order) and lists (numId, one per list in document order) across
the whole document, and wraps each heading's section in a bookmark. So the
render of a block alone is not a piece of the document's render. This
renderer makes it one:

1. split the old document.xml body into top-level items (paragraphs,
   tables, bookmark starts and ends);
2. render the old and the new block alone (pandoc docx) and take their
   items, bookmarks dropped;
3. find the old block's items in the old document (the same occurrence of
   the block in the md, bookmarks skipped, list ids ignored);
4. replace them with the new block's items, mapping each list id the
   block used alone to the one the document gave it;
5. for a heading, rename the section bookmark before it to the new
   heading's identifier (pandoc derives it from the heading text).

Checked per edit against a full re-render: document.xml byte-identical,
every other part of the package identical, and the whole .docx rebuilt
with the old package's entry order and compression.

Edits: E158's three per text (a word inserted after the first word of the
blocks at 1/4, 1/2, 3/4), plus one heading edit per text.

Run from packages/kernel: python3 scripts/docx_splice.py OUT.jsonl
"""

import io
import json
import os
import re
import sys
import struct
import time
import zipfile
import zlib

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from render_locality import FIX, blocks, edit_block, pandoc  # noqa: E402

TAG = re.compile(rb"<(/?)([A-Za-z0-9:]+)[^>]*?(/?)>")
BOOK = re.compile(rb"^<w:bookmark(Start|End)\b")
NUM = re.compile(rb'(<w:numId w:val=")(\d+)(")')
NAME = re.compile(rb'w:name="([^"]*)"')


def body(xml):
    a = xml.index(b"<w:body>") + len(b"<w:body>")
    z = xml.rindex(b"<w:sectPr")
    return a, z


def items(x):
    """Top-level elements of a body slice, as (start, end) offsets."""
    out, depth, start = [], 0, None
    for m in TAG.finditer(x):
        close, selfclose = m.group(1) == b"/", m.group(3) == b"/"
        if depth == 0 and not close:
            start = m.start()
        if selfclose:
            if depth == 0:
                out.append((start, m.end()))
            continue
        depth += -1 if close else 1
        if depth == 0:
            out.append((start, m.end()))
    return out


def content_items(xml):
    a, z = body(xml)
    x = xml[a:z]
    return [x[s:e] for s, e in items(x) if not BOOK.match(x[s:e])]


STYLE = re.compile(rb'<w:pStyle w:val="(FirstParagraph|BodyText)" />')


def norm(item):
    """An item with its list ids and its context style dropped: pandoc
    styles a paragraph FirstParagraph after a heading or at the start, and
    BodyText after another paragraph, so a block alone is always first."""
    return STYLE.sub(b"<S/>", NUM.sub(rb"\g<1>N\g<3>", item))


def style(item):
    m = STYLE.search(item)
    return m.group(1) if m else None


def nums(item):
    return [m.group(2) for m in NUM.finditer(item)]


def docx_xml(docx):
    return zipfile.ZipFile(io.BytesIO(docx)).read("word/document.xml")


def rezip(old_docx, new_xml):
    """The old package with document.xml replaced (or, given a dict, the
    parts it names), written entry by entry
    with the old package's order, flags, versions, times and deflate
    settings (pandoc's zip matches zlib's raw deflate at its default
    level; Python's zipfile would rewrite the flags)."""
    zin = zipfile.ZipFile(io.BytesIO(old_docx))
    new = new_xml if isinstance(new_xml, dict) else {"word/document.xml": new_xml}
    out, central = bytearray(), bytearray()
    for info in zin.infolist():
        data = new[info.filename] if info.filename in new else zin.read(info)
        if info.compress_type == zipfile.ZIP_DEFLATED:
            c = zlib.compressobj(zlib.Z_DEFAULT_COMPRESSION, zlib.DEFLATED, -15)
            packed = c.compress(data) + c.flush()
        else:
            packed = data
        name = info.filename.encode()
        y, mo, d, h, mi, se = info.date_time
        dtime, ddate = (h << 11) | (mi << 5) | (se // 2), ((y - 1980) << 9) | (mo << 5) | d
        crc = zlib.crc32(data)
        fields = (info.flag_bits, info.compress_type, dtime, ddate, crc, len(packed), len(data))
        offset = len(out)
        out += struct.pack("<IHHHHHIIIHH", 0x04034B50, info.extract_version, *fields,
                           len(name), len(info.extra)) + name + info.extra + packed
        central += struct.pack("<IHHHHHHIIIHHHHHII", 0x02014B50,
                               info.create_version | (info.create_system << 8),
                               info.extract_version, *fields, len(name), len(info.extra), 0,
                               0, info.internal_attr, info.external_attr, offset)
        central += name + info.extra
    end = struct.pack("<IHHHHIIH", 0x06054B50, 0, 0, len(zin.infolist()), len(zin.infolist()),
                      len(central), len(out), 0)
    return bytes(out + central + end)


def splice(old_xml, md_blocks, i, new_block, old_frag_xml):
    """old_xml with block i of the md replaced by new_block, or None.
    old_frag_xml: block i rendered alone (kept from when the old document
    was rendered)."""
    old_b = md_blocks[i]
    of = content_items(old_frag_xml)
    nf_docx = pandoc(new_block, "docx", False)[0]
    nf_xml = docx_xml(nf_docx)
    nf = content_items(nf_xml)
    if not of:
        return None
    a, z = body(old_xml)
    x = old_xml[a:z]
    spans = [(s, e) for s, e in items(x)]
    content = [k for k, (s, e) in enumerate(spans) if not BOOK.match(x[s:e])]
    want = [norm(it) for it in of]
    occurrence = sum(1 for b in md_blocks[:i] if b == old_b)
    found = -1
    for j in range(len(content) - len(want) + 1):
        if all(norm(x[spans[content[j + k]][0]:spans[content[j + k]][1]]) == want[k]
               for k in range(len(want))):
            if occurrence == 0:
                found = j
                break
            occurrence -= 1
    if found < 0:
        return None
    hit = [content[found + k] for k in range(len(want))]
    # List ids: the block alone numbers its lists from 1001; map them to the
    # document's.
    nmap = {}
    for k, it in enumerate(of):
        doc_it = x[spans[hit[k]][0]:spans[hit[k]][1]]
        for f, d in zip(nums(it), nums(doc_it)):
            nmap[f] = d
    new_items = [NUM.sub(lambda m: m.group(1) + nmap.get(m.group(2), m.group(2)) + m.group(3), it)
                 for it in nf]
    # Context styles: each new item takes the style its position had.
    for k, it in enumerate(new_items):
        doc_it = x[spans[hit[min(k, len(hit) - 1)]][0]:spans[hit[min(k, len(hit) - 1)]][1]]
        ds, ns = style(doc_it), style(it)
        if ds and ns and ds != ns:
            new_items[k] = it.replace(b'w:val="' + ns + b'"', b'w:val="' + ds + b'"', 1)
    s0, e0 = spans[hit[0]][0], spans[hit[-1]][1]
    # Bookmarks between the matched items stay where they were (after them).
    between = [x[s:e] for k, (s, e) in enumerate(spans)
               if hit[0] < k < hit[-1] and k not in hit]
    rebuilt = b"".join(new_items) + b"".join(between)
    prefix = x[:s0]
    # A heading: its section bookmark sits right before it; rename it.
    nb = [m.group(1) for m in NAME.finditer(nf_xml)]
    ob = [m.group(1) for m in NAME.finditer(old_frag_xml)]
    if nb and ob and nb != ob:
        k = hit[0] - 1
        if k >= 0 and x[spans[k][0]:spans[k][1]].startswith(b"<w:bookmarkStart"):
            bs, be = spans[k]
            mark = x[bs:be].replace(b'w:name="' + ob[0] + b'"', b'w:name="' + nb[0] + b'"')
            prefix = x[:bs] + mark + x[be:s0]
    return old_xml[:a] + prefix + rebuilt + x[e0:] + old_xml[z:]


def heading_edit(bs):
    for i, b in enumerate(bs):
        if b.startswith(b"## "):
            return i, b[:3] + b"Revised " + b[3:]
    return None


def main(out_path):
    names = sorted(n[:-3] for n in os.listdir(FIX) if n.endswith(".md"))
    out = open(out_path, "w")
    tot = dict(edits=0, xml_exact=0, parts_same=0, docx_exact=0, found=0)
    t_full = t_frag = 0.0
    for name in names:
        md = open(os.path.join(FIX, name + ".md"), "rb").read()
        bs = blocks(md)
        old_docx, _ = pandoc(md, "docx", False)
        old_xml = docx_xml(old_docx)
        edits = []
        for k, i in enumerate(sorted({len(bs) // 4, len(bs) // 2, 3 * len(bs) // 4})):
            nb = edit_block(bs[i])
            if nb is not None:
                edits.append(("word", i, nb))
        h = heading_edit(bs)
        if h:
            edits.append(("heading", h[0], h[1]))
        for kind, i, nb in edits:
            md2 = b"".join(bs[:i]) + nb + b"".join(bs[i + 1:])
            full, tf = pandoc(md2, "docx", False)
            old_frag = docx_xml(pandoc(bs[i], "docx", False)[0])
            t = time.perf_counter()
            got = splice(old_xml, bs, i, nb, old_frag)
            tg = time.perf_counter() - t
            full_zip = zipfile.ZipFile(io.BytesIO(full))
            want = full_zip.read("word/document.xml")
            old_zip = zipfile.ZipFile(io.BytesIO(old_docx))
            parts_same = all(full_zip.read(n) == old_zip.read(n)
                             for n in old_zip.namelist() if n != "word/document.xml")
            xml_exact = got == want
            docx_exact = xml_exact and rezip(old_docx, got) == full
            row = dict(text=name, kind=kind, block=i, found=got is not None,
                       xml_exact=xml_exact, parts_same=parts_same, docx_exact=docx_exact,
                       full_ms=round(tf * 1e3, 1), splice_ms=round(tg * 1e3, 1))
            if not xml_exact and got is not None:
                d = next((j for j in range(min(len(got), len(want))) if got[j] != want[j]),
                         min(len(got), len(want)))
                row["diff"] = [got[max(0, d - 60):d + 60].decode(errors="replace"),
                               want[max(0, d - 60):d + 60].decode(errors="replace")]
            out.write(json.dumps(row) + "\n")
            tot["edits"] += 1
            tot["found"] += got is not None
            tot["xml_exact"] += xml_exact
            tot["parts_same"] += parts_same
            tot["docx_exact"] += docx_exact
            t_full += tf
            t_frag += tg
    s = dict(summary=True, **tot, full_ms=round(t_full * 1e3), splice_ms=round(t_frag * 1e3))
    out.write(json.dumps(s) + "\n")
    print(json.dumps(s))


if __name__ == "__main__":
    main(sys.argv[1])
