from __future__ import annotations

import json
import random
from datetime import date
from pathlib import Path

CASE_DIR = Path(__file__).resolve().parent
PROMPTS_PATH = CASE_DIR / "assets/images/prompts.jsonl"


def main():
    rng = random.Random(22020605)
    created_at = str(date.today())

    targets = {"logos": 6, "people": 18, "clinics": 8, "diagrams": 10, "social": 6}
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
                "style_tags": ["mesa-ridge", "clinic-group", "blue-teal"],
                "source_case_id": "m-healthcare-clinic-v01",
            }
        )

    neg_photo = "watermark, blurry, low-res, real brand logos, readable addresses, readable patient data, celebrity, uncanny, distorted text"
    neg_flat = "photorealistic, 3D, bevel, embossed, mockup, watermark, stock logo"

    for i in range(1, targets["logos"] + 1):
        add("logos", i, "Flat vector logo for a clinic group named 'Mesa Ridge Care'. Blue and teal on white background, simple cross icon, no mockups.", neg_flat, 1024, 1024, 30, 5.5)

    roles=["medical director","ops manager","billing lead","HR manager","nurse","front desk staff"]
    for i in range(1, targets["people"] + 1):
        role=roles[(i-1)%len(roles)]
        add("people", i, f"Photoreal staff headshot of a {role} at a clinic group. Calm, professional, neutral background, clearly synthetic.", neg_photo, 1024, 1024, 32, 6.0)

    scenes=["clinic exterior in Denver suburb, no readable address","reception area with calm colors","exam room interior, no patient","billing office desks with blurred screens"]
    for i in range(1, targets["clinics"] + 1):
        add("clinics", i, f"Photoreal scene: {scenes[(i-1)%len(scenes)]}. Calm clinic vibe, no real brands, no PHI.", neg_photo, 1344, 768, 35, 6.0)

    diag=["billing process map","denials workflow","no-show definition comparison chart","collections definition comparison chart","training completion dashboard"]
    for i in range(1, targets["diagrams"] + 1):
        add("diagrams", i, f"Clean illustrative clinic ops diagram in blue/teal palette: {diag[(i-1)%len(diag)]}. Slide-ready, minimal text.", neg_flat, 1344, 768, 30, 5.5)

    for i in range(1, targets["social"] + 1):
        if i%2==0:
            prompt="Illustrative hiring post template for a clinic group: white background, blue header, teal accents, space for photo, minimal icons."
        else:
            prompt="Illustrative patient reminder template: appointment + privacy reassurance theme, blue/teal palette, clean layout."
        add("social", i, prompt, neg_flat, 1080, 1080, 28, 5.5)

    PROMPTS_PATH.write_text("\n".join(json.dumps(x, ensure_ascii=False) for x in lines) + "\n", encoding="utf-8")
    print(f"Wrote {len(lines)} prompts (target {sum(targets.values())}) -> {PROMPTS_PATH}")


if __name__ == "__main__":
    main()
