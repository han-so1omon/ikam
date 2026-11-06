"""Render the tables corpus (E100): each fixture case's first workbook
(alphabetically by path) as xlsx, csv, json, md, docx and pdf, the same
records in six formats. Run once by hand from this directory; the bench
reads only the committed files. Needs LibreOffice (soffice) and pandoc.

    python3 render.py
"""

import csv
import io
import json
import shutil
import subprocess
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
CASES = HERE.parents[3] / "tests" / "fixtures" / "cases"


def number(text):
    """A CSV cell as a JSON number when it reads as one, else the text."""
    for kind in (int, float):
        try:
            return kind(text)
        except ValueError:
            pass
    return text


def cell(text):
    return text.replace("|", "\\|")


def main():
    for case in sorted(p for p in CASES.iterdir() if p.is_dir() and not p.name.startswith("_")):
        books = sorted(case.rglob("*.xlsx"))
        if not books:
            continue
        book, name = books[0], case.name
        shutil.copyfile(book, HERE / f"{name}.xlsx")
        with tempfile.TemporaryDirectory() as tmp:
            # First sheet as UTF-8 CSV, formulas evaluated by LibreOffice.
            subprocess.run(
                ["soffice", "--headless", "--convert-to", "csv:Text - txt - csv (StarCalc):44,34,76,1",
                 "--outdir", tmp, str(HERE / f"{name}.xlsx")],
                check=True, capture_output=True,
            )
            text = (Path(tmp) / f"{name}.csv").read_text(encoding="utf-8")
        (HERE / f"{name}.csv").write_text(text, encoding="utf-8", newline="")
        rows = [r for r in csv.reader(io.StringIO(text)) if any(r)]
        header, records = rows[0], rows[1:]
        objects = [{k: number(v) for k, v in zip(header, r) if v != ""} for r in records]
        (HERE / f"{name}.json").write_text(
            json.dumps(objects, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        md = [f"# {book.stem}", "", f"From `{book.relative_to(CASES)}`, first sheet.", ""]
        md.append("| " + " | ".join(cell(h) for h in header) + " |")
        md.append("|" + "---|" * len(header))
        md += ["| " + " | ".join(cell(v) for v in r) + " |" for r in records]
        (HERE / f"{name}.md").write_text("\n".join(md) + "\n", encoding="utf-8")
        subprocess.run(["pandoc", f"{name}.md", "-o", f"{name}.docx"], cwd=HERE, check=True)
        subprocess.run(
            ["soffice", "--headless", "--convert-to", "pdf", f"{name}.docx"],
            cwd=HERE, check=True, capture_output=True,
        )


if __name__ == "__main__":
    main()
