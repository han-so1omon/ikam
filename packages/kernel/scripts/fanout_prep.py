"""E181, step 1: one paragraph shared by 64 md files, edited once and carried
to all 64 md files and their 64 html and 64 docx renders (192 endpoints).

Corpus: the 24 texts of fixtures/multiformat, cycled to 64 copies, each with
the same notice paragraph inserted before its middle block (as E160's
`fan/`). Renders: pandoc html (standalone, titled) and docx, as the
fixtures were made.

The edit revises the notice. Two ways to make the new renders:
- full: pandoc re-renders every file's html and docx (128 runs);
- splice: the notice's new html and docx are rendered once (pandoc runs on
  the block alone are memoized, as a renderer would cache them), then
  spliced into each file: html by E158's substring splice, docx by E169's
  renderer (each old document's block items are kept from ingest, which is
  not timed).
Every spliced file is checked byte-identical to its full re-render.

Writes OUT_DIR/{old,new}/<i>.{md,html,docx} and OUT_DIR/times.json, for
examples/fanout_endpoints.rs. Run from packages/kernel:
    python3 scripts/fanout_prep.py OUT_DIR
"""

import json
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import docx_restructure  # noqa: E402
import docx_splice  # noqa: E402
import render_locality  # noqa: E402
from render_locality import FIX, blocks  # noqa: E402

FAN = 64
NOTICE = (b"Notice: figures in this document are unaudited and may change after the "
          b"quarterly review.\n\n")
REVISED = (b"Notice: figures in this document are unaudited and may change after the "
           b"quarterly review (revised).\n\n")

_raw = render_locality.pandoc
_memo = {}


def memo_pandoc(md, fmt, standalone, title=None):
    k = (md, fmt, standalone, title)
    if k not in _memo:
        _memo[k] = _raw(md, fmt, standalone, title)
    return _memo[k][0], 0.0


def main(out_dir):
    names = sorted(n[:-3] for n in os.listdir(FIX) if n.endswith(".md"))
    old_md, titles = [], []
    for i in range(FAN):
        name = names[i % len(names)]
        md = open(os.path.join(FIX, name + ".md"), "rb").read()
        bs = blocks(md)
        k = len(bs) // 2
        old_md.append(b"".join(bs[:k]) + NOTICE + b"".join(bs[k:]))
        titles.append(name)
    for d in ("old", "new"):
        os.makedirs(os.path.join(out_dir, d), exist_ok=True)
    # Old renders and ingest-time state (not timed; cached renders).
    docx_restructure.pandoc = docx_splice.pandoc = memo_pandoc
    docs, old_html = [], []
    for i, md in enumerate(old_md):
        h = _raw(md, "html", True, title=titles[i])[0]
        docs.append(docx_restructure.Doc(md))
        old_html.append(h)
        for ext, data in (("md", md), ("html", h), ("docx", docs[i].docx)):
            open(os.path.join(out_dir, "old", f"{i}.{ext}"), "wb").write(data)
    new_md = [md.replace(NOTICE, REVISED, 1) for md in old_md]
    # Full re-render.
    t = time.perf_counter()
    full = [(_raw(md, "html", True, title=titles[i])[0], _raw(md, "docx", False)[0])
            for i, md in enumerate(new_md)]
    full_ms = (time.perf_counter() - t) * 1e3
    # Splice: the block's renders once, then 64 splices each.
    _memo.clear()
    t = time.perf_counter()
    old_frag = _raw(NOTICE, "html", False)[0].strip()
    new_frag = _raw(REVISED, "html", False)[0].strip()
    spliced = []
    for i, doc in enumerate(docs):
        assert old_html[i].count(old_frag) == 1
        h = old_html[i].replace(old_frag, new_frag, 1)
        k = doc.bs.index(NOTICE)
        spliced.append((h, docx_restructure.render(doc, k, k + 1, REVISED)))
    splice_ms = (time.perf_counter() - t) * 1e3
    exact = dict(html=0, docx=0)
    for i, ((fh, fd), (sh, sd)) in enumerate(zip(full, spliced)):
        exact["html"] += fh == sh
        exact["docx"] += fd == sd
        for ext, data in (("md", new_md[i]), ("html", sh), ("docx", sd)):
            open(os.path.join(out_dir, "new", f"{i}.{ext}"), "wb").write(data)
    s = dict(files=FAN, endpoints=3 * FAN, exact=exact, full_render_ms=round(full_ms),
             splice_ms=round(splice_ms), pandoc_runs_full=2 * FAN,
             pandoc_runs_splice=len(_memo) + 2)
    json.dump(s, open(os.path.join(out_dir, "times.json"), "w"))
    print(json.dumps(s))


if __name__ == "__main__":
    main(sys.argv[1])
