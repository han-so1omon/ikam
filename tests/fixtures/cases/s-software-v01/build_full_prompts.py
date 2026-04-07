from __future__ import annotations

import json
import random
from datetime import date
from pathlib import Path

CASE_DIR = Path(__file__).resolve().parent
PROMPTS_PATH = CASE_DIR / "assets/images/prompts.jsonl"


def main():
    rng = random.Random(170117)
    created_at = str(date.today())

    targets = {"logos": 4, "people": 10, "product-ui": 10, "diagrams": 8, "social": 4}
    lines = []

    def add(kind: str, idx: int, prompt: str, negative: str, w: int, h: int, steps: int, guidance: float):
        out_path = f"assets/images/{kind}/{kind}-{idx:03d}.png"
        if kind == "people":
            out_path = f"assets/images/people/person-{idx:03d}.png"
        elif kind == "product-ui":
            out_path = f"assets/images/product-ui/ui-{idx:03d}.png"
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
                "style_tags": ["pocketrelay", "micro-saas", "indigo-mint"],
                "source_case_id": "s-software-v01",
            }
        )

    neg_photo = "watermark, blurry, low-res, real brand logos, celebrity, uncanny, extra fingers, distorted text"
    neg_ui = "watermark, real brand logos, personal data, readable emails, phone numbers, credit cards, copyrighted UI"
    neg_flat = "photorealistic, 3D, bevel, embossed, mockup, watermark, stock logo"

    for i in range(1, targets["logos"] + 1):
        add("logos", i, "Flat vector logo for a micro-SaaS named 'PocketRelay'. Minimal relay/arrow icon, indigo with mint accent on off-white background. No mockups.", neg_flat, 1024, 1024, 28, 5.5)

    roles=["founder","engineer","support lead","finance contractor"]
    for i in range(1, targets["people"] + 1):
        role=roles[(i-1)%len(roles)]
        add("people", i, f"Photoreal headshot of a {role} at a tiny SaaS company. Neutral background, friendly expression, clearly synthetic.", neg_photo, 1024, 1024, 30, 6.0)

    ui=["pricing page with $49 and $99 tiers","routing rules editor","webhook settings page","queue depth chart","customer list page (placeholder names)"]
    for i in range(1, targets["product-ui"] + 1):
        add("product-ui", i, f"Synthetic UI screenshot for PocketRelay: {ui[(i-1)%len(ui)]}. Minimal micro-SaaS, indigo/mint palette, placeholder text only.", neg_ui, 1344, 768, 32, 6.0)

    diag=["churn definition diagram cancellations vs net revenue churn","pricing drift diagram pricing page vs legacy addendum","incident timeline for webhook retry storm","simple architecture diagram intake->queue->webhooks","KPI definitions card layout"]
    for i in range(1, targets["diagrams"] + 1):
        add("diagrams", i, f"Clean illustrative micro-SaaS diagram in indigo/mint palette: {diag[(i-1)%len(diag)]}. Slide-ready, minimal text.", neg_flat, 1344, 768, 28, 5.5)

    for i in range(1, targets["social"] + 1):
        add("social", i, "Illustrative social template for a micro-SaaS: off-white background, indigo header, mint accent line, space for UI screenshot, minimal icons.", neg_flat, 1080, 1080, 26, 5.5)

    PROMPTS_PATH.write_text("\n".join(json.dumps(x, ensure_ascii=False) for x in lines) + "\n", encoding="utf-8")
    print(f"Wrote {len(lines)} prompts (target {sum(targets.values())}) -> {PROMPTS_PATH}")


if __name__ == "__main__":
    main()
