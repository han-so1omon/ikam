from __future__ import annotations

import json
import random
from datetime import date
from pathlib import Path

CASE_DIR = Path(__file__).resolve().parent
PROMPTS_PATH = CASE_DIR / "assets/images/prompts.jsonl"


def main():
    rng = random.Random(55020605)
    created_at = str(date.today())

    targets = {"logos": 10, "people": 35, "products": 40, "stores": 15, "social": 25, "diagrams": 20}
    lines = []

    def add(kind: str, idx: int, prompt: str, negative: str, w: int, h: int, steps: int, guidance: float):
        out_path = f"assets/images/{kind}/{kind}-{idx:03d}.png"
        if kind == "people":
            out_path = f"assets/images/people/person-{idx:03d}.png"
        elif kind == "product":
            out_path = f"assets/images/products/product-{idx:03d}.png"
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
                "style_tags": ["sunbeam", "grocery", "multi-location", "yellow-green"],
                "source_case_id": "xl-local-retail-v01",
            }
        )

    neg_photo = "watermark, blurry, low-res, real brand logos, readable personal info, license plates, celebrity, uncanny, distorted text"
    neg_flat = "photorealistic, 3D, bevel, embossed, mockup, watermark, stock logo"

    # logos
    for i in range(1, targets["logos"] + 1):
        prompt = "Flat vector logo for a specialty grocery brand named 'Sunbeam Market Collective'. Sun-yellow and deep green on off-white background, simple sunburst + leaf icon, no mockups."
        add("logos", i, prompt, neg_flat, 1024, 1024, 30, 5.5)

    # people
    roles=["COO","CFO","marketing director","supply chain director","store manager","prepared foods lead","cashier","stock associate","commissary cook"]
    for i in range(1, targets["people"] + 1):
        role = roles[(i-1)%len(roles)]
        prompt = f"Photoreal staff headshot of a {role} at a specialty grocery company. Friendly, competent, natural light, clearly synthetic."
        add("people", i, prompt, neg_photo, 1024, 1024, 32, 6.0)

    # products
    items=["prepared foods bowl","fresh bakery loaf","produce display","catering tray","meal kit box","bottle of olive oil","salad jar"]
    for i in range(1, targets["products"] + 1):
        item = items[(i-1)%len(items)]
        prompt = f"Photoreal product photo of a {item} in a bright specialty grocery aesthetic, clean composition, no real brands, subtle sun-yellow/green props."
        add("products", i, prompt, neg_photo, 1024, 1024, 35, 6.0)

    # stores
    scenes=[
        "store interior with prepared foods counter and warm lighting",
        "store exterior street-level shot (no readable address)",
        "commissary kitchen prep line, clean, safety forward",
        "produce section with sunlit displays",
        "checkout area with minimal signage (unreadable)",
    ]
    for i in range(1, targets["stores"] + 1):
        scene=scenes[(i-1)%len(scenes)]
        prompt=f"Photoreal scene: {scene}. Bright friendly grocery vibe, no real brands, no readable personal data."
        add("stores", i, prompt, neg_photo, 1344, 768, 35, 6.0)

    # social
    for i in range(1, targets["social"] + 1):
        prompt="Illustrative social post template for a specialty grocer: off-white background, sun-yellow header, deep green accents, space for product photo, minimal icons."
        add("social", i, prompt, neg_flat, 1080, 1080, 28, 5.5)

    # diagrams
    diag=["store performance dashboard layout","waste vs shrink comparison chart","promo calendar timeline","inventory replenishment flow diagram","SOP version adoption tracker","labor scheduling heatmap","supply chain map"]
    for i in range(1, targets["diagrams"] + 1):
        prompt=f"Clean illustrative operations diagram in sun-yellow/deep-green palette: {diag[(i-1)%len(diag)]}. Slide-ready, minimal text."
        add("diagrams", i, prompt, neg_flat, 1344, 768, 30, 5.5)

    PROMPTS_PATH.write_text("\n".join(json.dumps(x, ensure_ascii=False) for x in lines) + "\n", encoding="utf-8")
    print(f"Wrote {len(lines)} prompts (target {sum(targets.values())}) -> {PROMPTS_PATH}")


if __name__ == "__main__":
    main()
