#!/usr/bin/env python3
"""E500: OfficeQA Pro V2 (Databricks) as a retrieval test for ikam. Turns the
dataset's question CSV and parsed JSONs into the files officeqa_bench.py
reads. The dataset is gated on Hugging Face (databricks/officeqa-pro-v2,
CC-BY-SA 4.0); nothing from it is committed.

    python3 -I prepare.py CSV JSON_DIR OUT_DIR [--embed wordllama]

CSV is officeqa_pro_v2.csv; JSON_DIR holds parsed_corpus/jsons/*.json (the
whole corpus, or only the documents the questions cite: the printed `docs`
says which). Writes to OUT_DIR:
- pages.tsv: `stem/page<TAB>text`, one line a PDF page of every document.
  Elements are joined in order; an HTML table becomes one line a row, its
  cells separated by ` | `. Newlines inside a page are stored as U+001E.
- queries.tsv: `uid<TAB>gold pages<TAB>gold docs<TAB>question`, the gold
  pages (`stem/page`, comma separated) from `source_docs` (corpus_file and
  pdf_page_number), the gold documents from `source_files` and `source_docs`.
- with --embed wordllama (needs the `wordllama` and `numpy` packages):
  pages.f32 and queries.f32, unit vectors (WordLlama's default l2_supercat,
  256 dimensions, as E358), and vectors.json.

Pages are numbered as the PDF's (1 first). The parse gives each element a
`page_id`; whether it counts from 0 or 1 is checked, not assumed: the base
whose gold pages hold more of their question's four-digit years is used, and
both counts are printed.
"""
import csv
import html.parser
import json
import os
import re
import sys

RS = "\x1e"


class Table(html.parser.HTMLParser):
    """An HTML table's text: one line a row, cells joined by ` | `."""

    def __init__(self):
        super().__init__()
        self.rows, self.row, self.cell = [], [], []

    def handle_starttag(self, tag, attrs):
        if tag == "tr":
            self.row = []
        elif tag in ("td", "th"):
            self.cell = []

    def handle_endtag(self, tag):
        if tag in ("td", "th"):
            self.row.append(" ".join("".join(self.cell).split()))
        elif tag == "tr":
            self.rows.append(" | ".join(self.row))

    def handle_data(self, data):
        self.cell.append(data)


def element_text(content):
    if "<table" in content.lower():
        t = Table()
        t.feed(content)
        return "\n".join(r for r in t.rows if r.strip())
    return content.strip()


def page_id(el):
    box = el.get("bbox")
    if isinstance(box, list) and box and isinstance(box[0], dict):
        return box[0].get("page_id")
    return None


def pages_of(path):
    """{page_id: text} of one parsed JSON (ai_parse_document's layout)."""
    with open(path, encoding="utf-8") as f:
        doc = json.load(f).get("document") or {}
    pages = {}
    for el in doc.get("elements") or []:
        content = el.get("content") if isinstance(el, dict) else None
        if not isinstance(content, str) or not content.strip():
            continue
        pid = page_id(el)
        pages.setdefault(0 if pid is None else int(pid), []).append(element_text(content))
    return {p: "\n".join(parts) for p, parts in pages.items()}


def numbers(spec):
    """`12`, `12, 13` or `12-14` as page numbers."""
    out = []
    for part in re.split(r"[,\s]+", spec.strip()):
        if re.fullmatch(r"\d+-\d+", part):
            a, b = map(int, part.split("-"))
            out.extend(range(a, b + 1))
        elif part.isdigit():
            out.append(int(part))
    return out


def stem(name):
    return os.path.splitext(name.strip())[0]


def gold(row):
    """The question's gold (stem, pdf page) pairs and gold documents."""
    pages = []
    for m in re.finditer(r"corpus_file=([^|;]+?)\s*\|\s*pdf_page_number=([0-9,\s-]+)",
                         row["source_docs"]):
        pages.extend((stem(m.group(1)), n) for n in numbers(m.group(2)))
    docs = {stem(n) for n in row["source_files"].split(";") if n.strip()}
    return pages, docs | {s for s, _ in pages}


def main():
    csv_path, json_dir, out_dir = sys.argv[1:4]
    embed = sys.argv[5] if sys.argv[4:5] == ["--embed"] else None
    os.makedirs(out_dir, exist_ok=True)
    rows = list(csv.DictReader(open(csv_path, encoding="utf-8")))
    golds = [gold(r) for r in rows]
    stems = sorted(stem(n) for n in os.listdir(json_dir) if n.endswith(".json"))
    docs = {s: pages_of(os.path.join(json_dir, s + ".json")) for s in stems}
    missing = sorted({d for _, ds in golds for d in ds} - set(docs))

    # The page base: gold pages holding their question's years.
    hits = {}
    for base in (0, 1):
        hits[base] = 0
        for r, (pages, _) in zip(rows, golds):
            years = set(re.findall(r"\b1[789]\d\d\b|\b20[0-2]\d\b", r["question"]))
            for s, n in pages:
                text = docs.get(s, {}).get(n - 1 + base, "")
                hits[base] += bool(years) and any(y in text for y in years)
    base = max(hits, key=hits.get)

    keys = []
    with open(os.path.join(out_dir, "pages.tsv"), "w", encoding="utf-8") as f:
        for s in stems:
            for pid, text in sorted(docs[s].items()):
                key = f"{s}/{pid + 1 - base}"
                keys.append(key)
                f.write(key + "\t" + text.replace("\t", " ").replace("\n", RS) + "\n")
    known = set(keys)
    found = total = 0
    with open(os.path.join(out_dir, "queries.tsv"), "w", encoding="utf-8") as f:
        for r, (pages, ds) in zip(rows, golds):
            gp = sorted({f"{s}/{n}" for s, n in pages})
            found += sum(k in known for k in gp)
            total += len(gp)
            q = " ".join(r["question"].split())
            f.write("\t".join([r["uid"], ",".join(gp), ",".join(sorted(ds)), q]) + "\n")
    print(json.dumps({"questions": len(rows), "docs": len(stems), "pages": len(keys),
                      "text_mb": round(sum(len(t.encode()) for d in docs.values()
                                           for t in d.values()) / 1e6, 1),
                      "gold_pages": total, "gold_pages_in_parse": found,
                      "gold_docs_missing": len(missing), "page_base": base,
                      "year_hits_by_base": hits}))
    if embed:
        write_vectors(embed, out_dir, keys, rows)


def write_vectors(model, out_dir, keys, rows):
    import numpy as np
    if model != "wordllama":
        raise SystemExit("--embed takes wordllama")
    from wordllama import WordLlama
    wl = WordLlama.load()  # l2_supercat, 256 dimensions (as E358)
    texts = [line.rstrip("\n").split("\t", 1)[1].replace(RS, "\n") for line in
             open(os.path.join(out_dir, "pages.tsv"), encoding="utf-8")]
    dim = 0
    for name, items in (("pages", texts), ("queries", [r["question"] for r in rows])):
        v = np.asarray(wl.embed(items, norm=False), dtype=np.float32)
        v /= np.maximum(np.linalg.norm(v, axis=1, keepdims=True), 1e-12)
        v.astype("<f4").tofile(os.path.join(out_dir, name + ".f32"))
        dim = v.shape[1]
    json.dump({"model": "wordllama l2_supercat", "dim": dim},
              open(os.path.join(out_dir, "vectors.json"), "w"))


if __name__ == "__main__":
    main()
