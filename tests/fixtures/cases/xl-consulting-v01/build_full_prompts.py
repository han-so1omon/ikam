from __future__ import annotations

import json
import random
from datetime import date
from pathlib import Path

CASE_DIR = Path(__file__).resolve().parent
PROMPTS_PATH = CASE_DIR / "assets/images/prompts.jsonl"


def main():
    rng = random.Random(202603)
    created_at = str(date.today())

    targets = {"logos": 8, "people": 24, "office": 14, "diagrams": 18, "social": 10}
    lines = []

    def add(kind: str, idx: int, prompt: str, negative: str, w: int, h: int, steps: int, guidance: float):
        out_path = f"assets/images/{kind}/{kind}-{idx:03d}.png"
        if kind == "people":
            out_path = f"assets/images/people/person-{idx:03d}.png"
        lines.append(
            {
                "id": f"{kind}-{idx:03d}",
                "out_path": out_path,
                "kind": kind,
                "prompt": prompt,
                "negative_prompt": negative,
                "model": "sdxl",
                "checkpoint": "SSD-1B.safetensors",
                "sampler": "dpmpp_2m",
                "steps": steps,
                "guidance": guidance,
                "width": w,
                "height": h,
                "seed": rng.randrange(10_000, 9_999_999),
                "created_at": created_at,
                "style_tags": ["alderpoint", "enterprise-consulting", "deep-blue-graphite-gold"],
                "source_case_id": "xl-consulting-v01",
            }
        )

    neg_photo = "watermark, blurry, low-res, real brand logos, readable client names, addresses, celebrity, uncanny, distorted text"
    neg_flat = "photorealistic, 3D, bevel, embossed, mockup, watermark, stock logo"

    for i in range(1, targets["logos"] + 1):
        add(
            "logos",
            i,
            "Flat vector logo for an enterprise consulting firm named 'Alderpoint Strategy & Operations'. Minimal compass/triangle mark, deep blue with graphite and subtle gold accent. No mockups.",
            neg_flat,
            1024,
            1024,
            30,
            5.5,
        )

    roles=["managing partner","program director","PMO lead","workstream lead","consultant","analyst","finance ops"]
    for i in range(1, targets["people"] + 1):
        role=roles[(i-1)%len(roles)]
        add("people", i, f"Photoreal headshot of a {role} at an enterprise consulting firm. Neutral background, clearly synthetic.", neg_photo, 1024, 1024, 32, 6.0)

    office=[
        "large consulting office lobby (no signage)",
        "workshop with sticky notes (text not readable)",
        "client meeting room with large screen (blurred content)",
        "consultant at laptop with slide deck open (text not readable)",
        "team standup around a program wall (unreadable)",
    ]
    for i in range(1, targets["office"] + 1):
        add("office", i, f"Photoreal scene: {office[(i-1)%len(office)]}. Enterprise consulting vibe, deep blue accents.", neg_photo, 1344, 768, 34, 6.0)

    diag=[
        "savings math diagram gross annualized vs net savings",
        "baseline dispute diagram and assumptions",
        "milestone date comparison 2026-03-15 vs 2026-03-29",
        "RAID log layout and workflow",
        "hours reconciliation diagram timesheet vs billed cap",
        "staffing plan heatmap layout",
        "deliverable index diagram versions across workstreams",
    ]
    for i in range(1, targets["diagrams"] + 1):
        add("diagrams", i, f"Clean illustrative consulting diagram in deep blue/graphite/white with subtle gold accent: {diag[(i-1)%len(diag)]}. Slide-ready, minimal text.", neg_flat, 1344, 768, 30, 5.5)

    for i in range(1, targets["social"] + 1):
        add("social", i, "Illustrative thought-leadership/hiring social template for an enterprise consulting firm: white background, deep blue header, graphite blocks, subtle gold accent line, space for chart.", neg_flat, 1080, 1080, 28, 5.5)

    PROMPTS_PATH.write_text("\n".join(json.dumps(x, ensure_ascii=False) for x in lines) + "\n", encoding="utf-8")
    print(f"Wrote {len(lines)} prompts (target {sum(targets.values())}) -> {PROMPTS_PATH}")


if __name__ == "__main__":
    main()
