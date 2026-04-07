from __future__ import annotations

import json
import random
from datetime import date
from pathlib import Path

CASE_DIR = Path(__file__).resolve().parent
PROMPTS_PATH = CASE_DIR / "assets/images/prompts.jsonl"


def main():
    rng = random.Random(55050605)
    created_at = str(date.today())

    targets = {"logos": 8, "people": 20, "site": 30, "diagrams": 18, "social": 10}
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
                "style_tags": ["summitspan", "construction", "regional-gc", "navy-orange"],
                "source_case_id": "xl-construction-v01",
            }
        )

    neg_photo = "watermark, blurry, low-res, real brand logos, readable addresses, license plates, celebrity, uncanny, distorted text"
    neg_flat = "photorealistic, 3D, bevel, embossed, mockup, watermark, stock logo"

    for i in range(1, targets["logos"] + 1):
        add("logos", i, "Flat vector logo for a large construction group named 'SummitSpan Builders Group'. Minimal bridge/mountain icon, navy with orange accent on off-white background. No mockups.", neg_flat, 1024, 1024, 30, 5.5)

    roles=["CEO","COO","VP precon","VP operations","controller","safety director","project manager","superintendent","project engineer"]
    for i in range(1, targets["people"] + 1):
        role=roles[(i-1)%len(roles)]
        add("people", i, f"Photoreal headshot of a {role} at a large construction company. Professional, neutral background, clearly synthetic.", neg_photo, 1024, 1024, 32, 6.0)

    site=["multifamily construction site exterior progress","interior framing and MEP rough-in","window installation prep","safety walk scene with PPE","concrete pour day","finish stage corridor","jobsite trailer meeting (unreadable board)"]
    for i in range(1, targets["site"] + 1):
        add("site", i, f"Photoreal construction progress photo: {site[(i-1)%len(site)]}. PNW vibe, no real brands.", neg_photo, 1344, 768, 35, 6.0)

    diag=["master schedule milestone comparison Jul vs Aug","CO approved vs pending totals dashboard","WIP margin chart PM vs accounting","RFI/submittal workflow diagram","lookahead schedule table layout","punchlist tracker layout","safety incident dashboard"]
    for i in range(1, targets["diagrams"] + 1):
        add("diagrams", i, f"Clean illustrative construction diagram in navy/orange palette: {diag[(i-1)%len(diag)]}. Slide-ready, minimal text.", neg_flat, 1344, 768, 30, 5.5)

    for i in range(1, targets["social"] + 1):
        prompt = "Illustrative recruiting/project update template for a large GC: off-white background, navy header bar, orange accent line, space for photo, minimal icons."
        add("social", i, prompt, neg_flat, 1080, 1080, 28, 5.5)

    PROMPTS_PATH.write_text("\n".join(json.dumps(x, ensure_ascii=False) for x in lines) + "\n", encoding="utf-8")
    print(f"Wrote {len(lines)} prompts (target {sum(targets.values())}) -> {PROMPTS_PATH}")


if __name__ == "__main__":
    main()
