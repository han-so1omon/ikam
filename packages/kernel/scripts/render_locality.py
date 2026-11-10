"""E158: does one edited paragraph change only its own piece of each render?

For each of the 24 texts in fixtures/multiformat (md, rendered by pandoc to
html and docx, and by LibreOffice to pdf), this measures:

- render locality: the share of the md's paragraph blocks (cut as
  `fragment::blocks` cuts them) whose render on its own (pandoc, no
  standalone wrapper) appears verbatim in the whole file's render, for html
  and for docx's word/document.xml body;
- three edits per text (a word inserted into the blocks at 1/4, 1/2, 3/4),
  each propagated two ways:
  - full: re-render the whole md (pandoc html, pandoc docx, LibreOffice pdf)
  - fragment: render only the edited block and splice it over the old
    block's render in the old html and document.xml
  and whether the fragment splice is byte-identical to the full re-render;
- the bytes the kernel writes to store each new render (ikam put into a
  store holding the old renders), and the tool times.

Run from packages/kernel after `cargo build --release`:
    python3 scripts/render_locality.py OUT.jsonl
Needs pandoc 3.1.3 and LibreOffice (soffice). SOURCE_DATE_EPOCH is fixed so
docx output is deterministic. Prints one JSON line per text and edit, and
a summary line.
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
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
KERNEL = os.path.dirname(HERE)
FIX = os.path.join(KERNEL, "fixtures", "multiformat")
IKAM = os.path.join(KERNEL, "target", "release", "ikam")
ENV = dict(os.environ, SOURCE_DATE_EPOCH="1767225600")


def blocks(b):
    """The kernel's paragraph blocks (fragment::blocks), for bytes b."""
    out, s, i, n = [], 0, 0, len(b)
    while i < n:
        if b[i] == 0x0A:
            j = i + 1
            blank = j < n and (b[j] == 0x0A or b[j:j + 2] == b"\r\n")
            if blank or j - s > 16 * 1024:
                while j < n and b[j] in (0x0A, 0x0D):
                    j += 1
                out.append(b[s:j])
                s = i = j
                continue
        i += 1
    if s < n:
        out.append(b[s:])
    return out


def pandoc(md, fmt, standalone, title=None):
    args = ["pandoc", "-f", "markdown", "-t", fmt, "-o", "-"]
    if standalone:
        args.append("-s")
    if title:
        args += ["--metadata", f"pagetitle={title}"]
    t = time.perf_counter()
    out = subprocess.run(args, input=md, capture_output=True, env=ENV, check=True).stdout
    return out, time.perf_counter() - t


def docx_body(docx):
    xml = zipfile.ZipFile(io.BytesIO(docx)).read("word/document.xml")
    a = xml.index(b"<w:body>") + len(b"<w:body>")
    z = xml.rindex(b"<w:sectPr")
    return xml, a, z


def store_size(d):
    return sum(os.path.getsize(os.path.join(r, f)) for r, _, fs in os.walk(d) for f in fs)


def put(store, data, suffix):
    with tempfile.NamedTemporaryFile(suffix=suffix, delete=False) as f:
        f.write(data)
    before = store_size(store)
    t = time.perf_counter()
    subprocess.run([IKAM, "--store", store, "put", f.name], capture_output=True, check=True)
    ms = (time.perf_counter() - t) * 1e3
    os.unlink(f.name)
    return store_size(store) - before, ms


def soffice_pdf(docx, work):
    p = os.path.join(work, "x.docx")
    with open(p, "wb") as f:
        f.write(docx)
    t = time.perf_counter()
    subprocess.run(["soffice", "--headless", "--convert-to", "pdf", "--outdir", work, p],
                   capture_output=True, env=ENV, check=True)
    dt = time.perf_counter() - t
    with open(os.path.join(work, "x.pdf"), "rb") as f:
        return f.read(), dt


def edit_block(blk):
    m = re.search(rb"\w+", blk)
    if not m:
        return None
    return blk[:m.end()] + b" revised" + blk[m.end():]


def main(out_path):
    names = sorted(n[:-3] for n in os.listdir(FIX) if n.endswith(".md"))
    work = tempfile.mkdtemp()
    store = os.path.join(work, "store")
    out = open(out_path, "w")
    tot = dict(blocks=0, html_local=0, html_local_bytes=0, bytes=0, docx_local=0,
               edits=0, html_exact=0, docx_exact=0)
    times = dict(full_html=0.0, full_docx=0.0, pdf=0.0, frag_html=0.0, frag_docx=0.0)
    written = dict(html=0, docx=0, pdf=0)
    for name in names:
        md = open(os.path.join(FIX, name + ".md"), "rb").read()
        html = open(os.path.join(FIX, name + ".html"), "rb").read()
        docx_old, _ = pandoc(md, "docx", False)
        xml_old, a_old, z_old = docx_body(docx_old)
        body_old = xml_old[a_old:z_old]
        for f in ("html", "docx", "pdf"):
            subprocess.run([IKAM, "--store", store, "put", os.path.join(FIX, f"{name}.{f}")],
                           capture_output=True, check=True)
        bs = blocks(md)
        local = 0
        for b in bs:
            frag, _ = pandoc(b, "html", False)
            fb, _ = pandoc(b, "docx", False)
            fx, fa, fz = docx_body(fb)
            tot["blocks"] += 1
            tot["bytes"] += len(b)
            if frag.strip() and frag.strip() in html:
                tot["html_local"] += 1
                tot["html_local_bytes"] += len(b)
                local += 1
            if fx[fa:fz] and fx[fa:fz] in body_old:
                tot["docx_local"] += 1
        targets = sorted({len(bs) // 4, len(bs) // 2, 3 * len(bs) // 4})
        for k, i in enumerate(targets):
            old_b = bs[i]
            new_b = edit_block(old_b)
            if new_b is None:
                continue
            md2 = b"".join(bs[:i]) + new_b + b"".join(bs[i + 1:])
            full_html, t1 = pandoc(md2, "html", True, title=name)
            full_docx, t2 = pandoc(md2, "docx", False)
            # Fragment path: the edited block alone, spliced over the old one.
            of, _ = pandoc(old_b, "html", False)
            nf, t3 = pandoc(new_b, "html", False)
            o, n = of.strip(), nf.strip()
            html_exact = o in html and html.replace(o, n, 1) == full_html
            ofd, _ = pandoc(old_b, "docx", False)
            nfd, t4 = pandoc(new_b, "docx", False)
            ox, oa, oz = docx_body(ofd)
            nx, na, nz = docx_body(nfd)
            xml_new, _, _ = docx_body(full_docx)
            spliced = xml_old.replace(ox[oa:oz], nx[na:nz], 1) if ox[oa:oz] in body_old else None
            docx_exact = spliced == xml_new
            row = dict(text=name, edit=k, block=i, block_bytes=len(old_b),
                       html_exact=html_exact, docx_xml_exact=docx_exact,
                       full_html_ms=round(t1 * 1e3, 1), full_docx_ms=round(t2 * 1e3, 1),
                       frag_html_ms=round(t3 * 1e3, 1), frag_docx_ms=round(t4 * 1e3, 1))
            times["full_html"] += t1
            times["full_docx"] += t2
            times["frag_html"] += t3
            times["frag_docx"] += t4
            if k == 0:
                pdf, t5 = soffice_pdf(full_docx, work)
                times["pdf"] += t5
                row["pdf_ms"] = round(t5 * 1e3, 1)
                w, ms = put(store, pdf, ".pdf")
                row["pdf_written"] = w
                written["pdf"] += w
            w, _ = put(store, full_html, ".html")
            row["html_written"] = w
            written["html"] += w
            w, _ = put(store, full_docx, ".docx")
            row["docx_written"] = w
            written["docx"] += w
            tot["edits"] += 1
            tot["html_exact"] += html_exact
            tot["docx_exact"] += docx_exact
            out.write(json.dumps(row) + "\n")
            out.flush()
        print(name, local, len(bs), file=sys.stderr)
    summary = dict(summary=True, **tot,
                   ms={k: round(v * 1e3, 0) for k, v in times.items()},
                   written=written, pdf_edits=len(names))
    out.write(json.dumps(summary) + "\n")
    print(json.dumps(summary))
    shutil.rmtree(work)


if __name__ == "__main__":
    main(sys.argv[1])
