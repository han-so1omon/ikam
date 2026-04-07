"""Add generated .docx fixtures to each case's idea.md artifact list.

We insert a section before the "## Timeline" header:

### Word documents (.docx)
- <filename>

Idempotent: if the section already exists, it is replaced with current list.
"""

import os
import re
from pathlib import Path

BASE = Path(__file__).resolve().parent

SKIP = {"_image_tools"}

# Normalize any existing Word-doc sections to this canonical heading
SECTION_TITLE = "### Word documents (.docx)"


def case_dirs():
    for p in sorted(BASE.iterdir()):
        if not p.is_dir():
            continue
        if p.name.startswith("_"):
            continue
        if p.name in SKIP:
            continue
        yield p


def build_section(docx_files):
    lines = [SECTION_TITLE, ""]
    for f in sorted(docx_files):
        lines.append(f"- {f}")
    lines.append("")
    return "\n".join(lines)


def upsert_section(text, section_md):
    # replace existing section(s) if present (handle older heading variants)
    patterns = [
        re.compile(rf"\n{re.escape(SECTION_TITLE)}\n(.*?)(?=\n### |\n## |\Z)", re.S),
        re.compile(r"\n### Word documents\n(.*?)(?=\n### |\n## |\Z)", re.S),
    ]
    replaced = False
    for pat in patterns:
        if pat.search(text):
            text = pat.sub("\n" + section_md.rstrip() + "\n", text)
            replaced = True

    if replaced:
        return text

    # otherwise insert before Timeline header if possible
    timeline_re = re.compile(r"\n## Timeline", re.S)
    m = timeline_re.search(text)
    if m:
        return text[: m.start()] + "\n" + section_md.rstrip() + "\n" + text[m.start():]

    # else append at end
    return text.rstrip() + "\n\n" + section_md


def main():
    changed = 0
    for case in case_dirs():
        idea = case / "idea.md"
        if not idea.exists():
            continue
        docx_files = [p.name for p in case.glob("*.docx")]
        if not docx_files:
            continue

        original = idea.read_text(encoding="utf-8")
        section_md = build_section(docx_files)
        updated = upsert_section(original, section_md)

        if updated != original:
            idea.write_text(updated, encoding="utf-8")
            changed += 1
            print(f"updated: {case.name}/idea.md")

    print(f"done. updated {changed} idea.md files")


if __name__ == "__main__":
    main()
