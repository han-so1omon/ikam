from __future__ import annotations

from typing import Any, Dict

from .enwiki8_loader import load_enwiki8_bytes


def _make_text_block(seed: dict, label: str, size: str) -> str:
    base = f"{seed['name']} — {seed['tagline']} ({label})"
    if size == "small":
        return base
    if size == "medium":
        return f"{base}\nSegment: {seed['segment']}\nFocus: retention + CAC"
    return (
        f"{base}\nSegment: {seed['segment']}\n"
        "Focus: retention + CAC\nAssumptions: 3-year plan"
    )


def generate_project(
    seed: dict,
    size: str = "small",
    custom_prompt: str | None = None,
) -> Dict[str, Any]:
    doc_text = custom_prompt or _make_text_block(seed, "brief", size)
    sheet_rows = 5 if size == "small" else 12 if size == "medium" else 20
    slides = 4 if size == "small" else 8 if size == "medium" else 12
    threads = 2 if size == "small" else 4 if size == "medium" else 6
    plans = 1 if size == "small" else 2 if size == "medium" else 3

    project = {
        "seed": seed,
        "size": size,
        "docs": [
            {
                "title": f"{seed['name']} Overview",
                "body": doc_text,
            }
        ],
        "sheets": [
            {
                "name": f"{seed['name']} Model",
                "rows": [
                    {"label": f"Line {idx + 1}", "value": idx * 1000}
                    for idx in range(sheet_rows)
                ],
            }
        ],
        "slides": [
            {"title": f"Slide {idx + 1}", "notes": doc_text}
            for idx in range(slides)
        ],
        "threads": [
            {"topic": f"Thread {idx + 1}", "messages": [doc_text]}
            for idx in range(threads)
        ],
        "plans": [
            {"milestone": f"Milestone {idx + 1}", "summary": doc_text}
            for idx in range(plans)
        ],
    }

    if size == "large":
        try:
            project["enwiki8"] = load_enwiki8_bytes()
        except FileNotFoundError:
            pass

    return project
