from __future__ import annotations

import json
import random
from datetime import date
from pathlib import Path

CASE_DIR = Path(__file__).resolve().parent
PROMPTS_PATH = CASE_DIR / "assets/images/prompts.jsonl"


def main():
    rng = random.Random(202509)
    created_at = str(date.today())

    targets = {"logos": 4, "people": 10, "office": 8, "diagrams": 8, "social": 4}
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
                "style_tags": ["birchline", "boutique-consulting", "forest-slate-cream"],
                "source_case_id": "s-consulting-v01",
            }
        )

    neg_photo = "watermark, blurry, low-res, real brand logos, readable client names, addresses, celebrity, uncanny, distorted text"
    neg_flat = "photorealistic, 3D, bevel, embossed, mockup, watermark, stock logo"

    for i in range(1, targets["logos"] + 1):
        add("logos", i, "Flat vector logo for a boutique consulting firm named 'Birchline Insights'. Simple birch leaf/line icon, forest green with slate and cream. No mockups.", neg_flat, 1024, 1024, 28, 5.5)

    roles=["principal consultant","consultant","analyst","fractional finance lead"]
    for i in range(1, targets["people"] + 1):
        role=roles[(i-1)%len(roles)]
        add("people", i, f"Photoreal headshot of a {role} at a boutique consulting firm. Neutral background, clearly synthetic.", neg_photo, 1024, 1024, 30, 6.0)

    office=["small modern office with whiteboard (text not readable)","remote work setup laptop and notebook","team workshop around table (no readable text)","presentation rehearsal in small meeting room"]
    for i in range(1, targets["office"] + 1):
        add("office", i, f"Photoreal scene: {office[(i-1)%len(office)]}. Boutique consulting vibe, forest/slate accents.", neg_photo, 1344, 768, 32, 6.0)

    diag=["scope diagram discovery vs implementation","hours reconciliation diagram timesheet vs billed with write-offs","project plan timeline 6 weeks","deliverable status diagram FINAL vs draft email note","simple pipeline funnel chart"]
    for i in range(1, targets["diagrams"] + 1):
        add("diagrams", i, f"Clean illustrative consulting diagram in forest/slate/cream palette: {diag[(i-1)%len(diag)]}. Slide-ready, minimal text.", neg_flat, 1344, 768, 28, 5.5)

    for i in range(1, targets["social"] + 1):
        add("social", i, "Illustrative social template for a boutique consulting firm: cream background, forest header, slate text blocks, space for chart, minimal icons.", neg_flat, 1080, 1080, 26, 5.5)

    PROMPTS_PATH.write_text("\n".join(json.dumps(x, ensure_ascii=False) for x in lines) + "\n", encoding="utf-8")
    print(f"Wrote {len(lines)} prompts (target {sum(targets.values())}) -> {PROMPTS_PATH}")


if __name__ == "__main__":
    main()
