"""E167, step 1: one md paragraph edit carried to the text's html and docx
renders, by fragments, for the kernel measurement in examples/endpoints.rs.

For each of the 24 texts in fixtures/multiformat and E158's three edits:

- md: the new md (one block replaced);
- html: the old html cut into pieces (each md block's own html where it is
  a piece of the document's html, E158, and the gaps between), and the new
  html made by splicing the edited block's html alone (pandoc) over the old
  piece; checked against a full re-render;
- docx: E163's splice, checked against a full re-render.

Writes OUT_DIR/<text>/<edit>/ with old.md, new.md, old.html, new.html,
old.html.cuts and new.html.cuts (JSON offsets), old.docx, new.docx, and
times.json (ms for the full re-renders and for the fragment renders and
splices). Run from packages/kernel: python3 scripts/endpoints_prep.py OUT_DIR
"""

import json
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from docx_splice import docx_xml, rezip, splice  # noqa: E402
from render_locality import FIX, blocks, edit_block, pandoc  # noqa: E402


def html_cuts(html, bs):
    """Piece boundaries of html: each block's html where found in order."""
    cuts, cur, spans = [0], 0, {}
    for i, b in enumerate(bs):
        f = pandoc(b, "html", False)[0].strip()
        if not f:
            continue
        at = html.find(f, cur)
        if at < 0:
            continue
        spans[i] = (at, at + len(f))
        cuts += [at, at + len(f)]
        cur = at + len(f)
    cuts.append(len(html))
    cuts = sorted(set(cuts))
    return cuts, spans


def main(out_dir):
    names = sorted(n[:-3] for n in os.listdir(FIX) if n.endswith(".md"))
    ok = dict(html=0, docx=0, edits=0)
    for name in names:
        md = open(os.path.join(FIX, name + ".md"), "rb").read()
        html = open(os.path.join(FIX, name + ".html"), "rb").read()
        bs = blocks(md)
        cuts, spans = html_cuts(html, bs)
        old_docx = pandoc(md, "docx", False)[0]
        old_xml = docx_xml(old_docx)
        for k, i in enumerate(sorted({len(bs) // 4, len(bs) // 2, 3 * len(bs) // 4})):
            nb = edit_block(bs[i])
            if nb is None or i not in spans:
                continue
            md2 = b"".join(bs[:i]) + nb + b"".join(bs[i + 1:])
            t = {}
            s = time.perf_counter()
            full_html = pandoc(md2, "html", True, title=name)[0]
            full_docx = pandoc(md2, "docx", False)[0]
            t["full_render_ms"] = (time.perf_counter() - s) * 1e3
            old_frag = docx_xml(pandoc(bs[i], "docx", False)[0])  # kept from ingest
            s = time.perf_counter()
            a, z = spans[i]
            nf = pandoc(nb, "html", False)[0].strip()
            new_html = html[:a] + nf + html[z:]
            new_docx = rezip(old_docx, splice(old_xml, bs, i, nb, old_frag))
            t["fragment_render_ms"] = (time.perf_counter() - s) * 1e3
            d = len(nf) - (z - a)
            new_cuts = [c if c <= a else c + d for c in cuts]
            new_cuts = sorted(set(new_cuts))
            ok["edits"] += 1
            ok["html"] += new_html == full_html
            ok["docx"] += new_docx == full_docx
            assert new_html == full_html and new_docx == full_docx, (name, i)
            e = os.path.join(out_dir, name, str(k))
            os.makedirs(e, exist_ok=True)
            for fn, data in [("old.md", md), ("new.md", md2), ("old.html", html),
                             ("new.html", new_html), ("old.docx", old_docx),
                             ("new.docx", new_docx)]:
                open(os.path.join(e, fn), "wb").write(data)
            json.dump(cuts, open(os.path.join(e, "old.html.cuts"), "w"))
            json.dump(new_cuts, open(os.path.join(e, "new.html.cuts"), "w"))
            json.dump(t, open(os.path.join(e, "times.json"), "w"))
    print(json.dumps(ok))


if __name__ == "__main__":
    main(sys.argv[1])
