from __future__ import annotations

import json
import random
from datetime import date
from pathlib import Path

CASE_DIR = Path(__file__).resolve().parent
PROMPTS_PATH = CASE_DIR / "assets/images/prompts.jsonl"


def main():
    rng = random.Random(33020605)
    created_at = str(date.today())

    targets = {"logos": 6, "people": 18, "products": 25, "stores": 6, "social": 12, "diagrams": 10}
    lines = []

    def add(kind: str, idx: int, prompt: str, negative: str, w: int, h: int, steps: int, guidance: float):
        out_path = f"assets/images/{kind}/{kind}-{idx:03d}.png"
        if kind == "people":
            out_path = f"assets/images/people/person-{idx:03d}.png"
        elif kind == "products":
            out_path = f"assets/images/products/product-{idx:03d}.png"
        elif kind == "stores":
            out_path = f"assets/images/stores/store-{idx:03d}.png"
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
                "style_tags": ["driftwood-oak", "boutique", "oak-cream-blue"],
                "source_case_id": "m-local-retail-v01",
            }
        )

    neg_photo = "watermark, blurry, low-res, real brand logos, readable personal info, license plates, celebrity, uncanny, distorted text"
    neg_flat = "photorealistic, 3D, bevel, embossed, mockup, watermark, stock logo"

    for i in range(1, targets["logos"] + 1):
        add("logos", i, "Flat vector logo for a boutique named 'Driftwood & Oak'. Oak brown and muted blue accents on cream background, minimal oak leaf icon, no mockups.", neg_flat, 1024, 1024, 30, 5.5)

    roles=["founder","ops lead","merch lead","marketing lead","store manager","retail associate","fulfillment lead"]
    for i in range(1, targets["people"] + 1):
        role=roles[(i-1)%len(roles)]
        add("people", i, f"Photoreal staff headshot of a {role} at a boutique retail company. Warm natural light, friendly and competent, clearly synthetic.", neg_photo, 1024, 1024, 32, 6.0)

    items=["ceramic mug","oak scented candle","linen towel","stationery set","gift bundle box","minimal vase"]
    for i in range(1, targets["products"] + 1):
        item=items[(i-1)%len(items)]
        add("products", i, f"Photoreal product photo of a {item} in a warm boutique aesthetic, cream background, subtle oak/brown/blue props, no real brands.", neg_photo, 1024, 1024, 35, 6.0)

    scenes=["boutique store interior with warm lighting and shelves","second location exterior street-level (no readable address)","small fulfillment backroom with packing table"]
    for i in range(1, targets["stores"] + 1):
        scene=scenes[(i-1)%len(scenes)]
        add("stores", i, f"Photoreal scene: {scene}. Warm boutique vibe, no real brands, no readable personal data.", neg_photo, 1344, 768, 35, 6.0)

    for i in range(1, targets["social"] + 1):
        add("social", i, "Illustrative social template for a boutique: cream background, oak brown header bar, muted blue accent line, space for product photo, minimal icons.", neg_flat, 1080, 1080, 28, 5.5)

    diag=["promo calendar timeline","inventory adjustment flow diagram","gross vs net sales comparison chart","store performance dashboard layout","replenishment process flow"]
    for i in range(1, targets["diagrams"] + 1):
        add("diagrams", i, f"Clean illustrative retail ops diagram in oak/cream/muted-blue palette: {diag[(i-1)%len(diag)]}. Slide-ready, minimal text.", neg_flat, 1344, 768, 30, 5.5)

    PROMPTS_PATH.write_text("\n".join(json.dumps(x, ensure_ascii=False) for x in lines) + "\n", encoding="utf-8")
    print(f"Wrote {len(lines)} prompts (target {sum(targets.values())}) -> {PROMPTS_PATH}")


if __name__ == "__main__":
    main()
